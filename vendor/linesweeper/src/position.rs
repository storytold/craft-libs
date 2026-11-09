//! An algorithm for laying out curves in a given order.
//!
//! Our boolean ops algorithms work by first deciding horizontal orderings for curves
//! (with some guarantees about being approximately correct)
//! and then laying out the curves in a way that has the horizontal orders we decided
//! on. For this second step, which is the task of this module, we really
//! guarantee correctness: the curves that we output really have the specified
//! orders.

use std::collections::BinaryHeap;

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest, QuadBez};
use polycool::Quadratic;

use crate::{
    SegIdx, Segment, Segments,
    curve::{
        Order, QuadraticApprox, solve_t_for_y,
        transverse::{transversal_after, transversal_before},
        y_subsegment,
    },
    num::CheapOrderedFloat,
    order::ComparisonCache,
    topology::{HalfOutputSegVec, OutputSegIdx, OutputSegVec, ScanLineOrder},
};

// TODO: maybe a Context type to keep the parameters in check?
#[allow(clippy::too_many_arguments)]
fn ordered_curves_all_close(
    segs: &Segments,
    order: &[SegIdx],
    output_order: &[OutputSegIdx],
    out: &mut OutputSegVec<PositionedOutputSeg>,
    mut y0: f64,
    y1: f64,
    endpoints: &HalfOutputSegVec<kurbo::Point>,
    accuracy: f64,
) {
    if order.iter().all(|&s| segs[s].is_line()) {
        // If everything is a line, since our endpoints are already in the right order
        // the segments between them are also guaranteed to be in the right order.
        return;
    }

    if order.len() == 2 {
        let s0 = order[0];
        let s1 = order[1];
        let o0 = output_order[0];
        let o1 = output_order[1];

        let p0 = bez_end(&out[o0].path);
        let p1 = bez_end(&out[o1].path);
        let q0 = endpoints[o0.second_half()];
        let q1 = endpoints[o1.second_half()];

        if p0.y == y0 && p1.y == y0 {
            let c0 = next_subsegment(&segs[s0], &out[o0].path, y1, q0);
            let c1 = next_subsegment(&segs[s1], &out[o1].path, y1, q1);

            if transversal_after(c0, c1, y1) {
                return;
            }
        }

        if q0.y == y1 && q1.y == y1 {
            let c0 = next_subsegment(&segs[s0], &out[o0].path, y1, q0);
            let c1 = next_subsegment(&segs[s1], &out[o1].path, y1, q1);

            if transversal_before(c0, c1, y0) {
                return;
            }
        }
    }

    // Ensure everything in `out` goes up to `y0`. Anything that doesn't go up to `y0` is
    // an output where we can just copy from the input.
    for (&seg_idx, &out_idx) in order.iter().zip(output_order) {
        let out = &mut out[out_idx];
        if bez_end_y(&out.path) < y0 {
            out.copied_idx = Some(out.path.elements().len() - 1);
            let endpoint = endpoints[out_idx.second_half()];
            let c = next_subsegment(&segs[seg_idx], &out.path, y0, endpoint);
            if segs[seg_idx].is_line() {
                out.path.line_to(c.p3)
            } else {
                out.path.curve_to(c.p1, c.p2, c.p3);
            }
        }
    }

    let mut approxes = Vec::new();

    // The recentering helps, but there are still some issues with approximation when almost horizontal
    while y0 < y1 {
        let next_y0 = approximate(y0, y1, order, segs, accuracy, &mut approxes);

        let y_mid = (y0 + next_y0) / 2.0;

        let mut x_mid_max_so_far = f64::NEG_INFINITY;
        let mut x1_max_so_far = f64::NEG_INFINITY;
        for (quad, out_idx) in approxes.iter().zip(output_order) {
            let &[c0, c1, c2] = quad.q.coeffs();
            // These would be the control points, but since `approximate` recenters the y interval,
            // they're different...
            // let x_mid = c0 + y_mid * c1 + y0 * next_y0 * c2;
            // let x1 = c0 + c1 * next_y0 + c2 * next_y0 * next_y0;
            let x_mid = c0 + (y0 - y_mid) * (next_y0 - y_mid) * c2;
            let x1 = c0 + c1 * (next_y0 - y_mid) + c2 * (next_y0 - y_mid) * (next_y0 - y_mid);

            x_mid_max_so_far = x_mid_max_so_far.max(x_mid);
            x1_max_so_far = x1_max_so_far.max(x1);

            out[*out_idx]
                .path
                .quad_to((x_mid_max_so_far, y_mid), (x1_max_so_far, next_y0));
        }
        approxes.clear();

        y0 = next_y0;
    }
}

fn horizontal_error_weight(c: CubicBez) -> f64 {
    let tangents = [c.p1 - c.p0, c.p2 - c.p1, c.p3 - c.p2];
    let mut weight = 0.0f64;

    fn different_signs(x: f64, y: f64) -> bool {
        (x > 0.0 && y < 0.0) || (x < 0.0) && (y > 0.0)
    }

    if different_signs(tangents[0].x, tangents[1].x)
        || different_signs(tangents[1].x, tangents[2].x)
    {
        return 1.0;
    }

    for v in tangents {
        let denom = v.hypot2().sqrt();
        if denom != 0.0 {
            let w = v.y.abs() / denom;
            weight = weight.max(w);
        }
    }
    weight
}

// Pushes approximating parabolas into `out`, and returns the final y position.
fn approximate(
    y0: f64,
    mut y1: f64,
    order: &[SegIdx],
    segs: &Segments,
    accuracy: f64,
    out: &mut Vec<QuadraticApprox>,
) -> f64 {
    let orig_len = out.len();

    'retry: loop {
        let y_mid = (y0 + y1) / 2.0;
        for seg_idx in order {
            let cubic = segs[*seg_idx].to_kurbo_cubic();
            let mut cubic = y_subsegment(cubic, y0, y1);
            cubic.p0.y -= y_mid;
            cubic.p1.y -= y_mid;
            cubic.p2.y -= y_mid;
            cubic.p3.y -= y_mid;
            let approx = QuadraticApprox::from_cubic(cubic, None);

            let factor = horizontal_error_weight(cubic);

            let too_short = y1 <= y0 + accuracy;
            let accurate = approx.dmax.max(-approx.dmin) * factor < accuracy;
            if !too_short && !accurate {
                // TODO: can we be smarter about this?
                y1 = (y0 + y1) / 2.0;
                out.truncate(orig_len);
                continue 'retry;
            }

            out.push(approx);
        }

        return y1;
    }
}

#[derive(Debug, PartialEq, Eq)]
struct HeapEntry {
    y: CheapOrderedFloat,
    idx: OutputSegIdx,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.y.cmp(&self.y).then(self.idx.cmp(&other.idx))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn bez_end(path: &BezPath) -> kurbo::Point {
    match path.elements().last().unwrap() {
        kurbo::PathEl::MoveTo(p)
        | kurbo::PathEl::LineTo(p)
        | kurbo::PathEl::QuadTo(_, p)
        | kurbo::PathEl::CurveTo(_, _, p) => *p,
        kurbo::PathEl::ClosePath => unreachable!(),
    }
}

fn bez_end_y(path: &BezPath) -> f64 {
    bez_end(path).y
}

fn next_subsegment(seg: &Segment, out: &BezPath, y1: f64, endpoint: kurbo::Point) -> CubicBez {
    let p0 = bez_end(out);
    let orig = seg.to_kurbo_cubic();
    let mut c = y_subsegment(orig, p0.y, y1);
    let p3 = if endpoint.y == y1 { endpoint } else { c.p3 };

    // Craft patch (flat-end shear)
    if (p0 != c.p0 || p3 != c.p3)
        && let Some(piece) = settled_cubic(
            orig,
            solve_t_for_y(orig, p0.y),
            solve_t_for_y(orig, y1),
            p0,
            p3,
        )
    {
        return piece;
    }

    c.p0 = p0;
    c.p3 = p3;
    c
}

// Craft patch (flat-end shear): where a curve is almost horizontal at an end, a
// tiny error in that end's height is a large error in the parameter that
// `solve_t_for_y` returns, and so a large horizontal gap between the cut and
// the end point. Moving the inner control points by their height's share of
// that gap moves the steep part of the curve sideways, and moving only the end
// point, as `next_subsegment` did, bends the curve near it. `settled_cubic` cuts
// the curve at the points nearest the end points instead, moves each end
// sideways with its handle and maps the heights linearly, so the piece moves
// no further than its ends do and stays monotonic in `y`.

/// The parameter between `lo` and `hi` of the point of `c` nearest `target`, found by
/// Newton steps from `t`, restarting once from the nearest point of the whole curve where a
/// step cannot leave, such as at a repeated handle. Returns `t` when no closer point is found.
fn settle_end(c: CubicBez, t: f64, target: kurbo::Point, lo: f64, hi: f64) -> f64 {
    let d1 = c.deriv();
    let d2 = d1.deriv();
    let mut best = t;
    let mut best_gap = (c.eval(t) - target).hypot2();
    let mut s = t;
    let mut restarted = false;
    for _ in 0..32 {
        if best_gap == 0.0 {
            break;
        }
        let r = c.eval(s) - target;
        let v = d1.eval(s).to_vec2();
        let a = d2.eval(s).to_vec2();
        let slope = r.dot(v);
        let bend = v.hypot2() + r.dot(a);
        // A Newton step cannot leave a point where the derivative is zero, such
        // as a repeated handle at an end. Restart once from the nearest point of
        // the whole curve, within the bounds, and keep it only if it is closer.
        if bend <= 0.0 || bend.is_nan() || v.hypot2() == 0.0 {
            let near = c.nearest(target, 1e-12).t.clamp(lo, hi);
            let gap = (c.eval(near) - target).hypot2();
            if restarted || gap >= best_gap {
                break;
            }
            restarted = true;
            best = near;
            best_gap = gap;
            s = near;
            continue;
        }
        let next = (s - slope / bend).clamp(lo, hi);
        if !next.is_finite() || next == s {
            break;
        }
        s = next;
        let gap = (c.eval(s) - target).hypot2();
        if gap < best_gap {
            best = s;
            best_gap = gap;
        }
    }
    best
}

/// The parameters of the points of `c` nearest `p0` and `p3`, searching from `t0` and
/// `t3`, or `None` when they are not in order.
fn settled_range(
    c: CubicBez,
    t0: f64,
    t3: f64,
    p0: kurbo::Point,
    p3: kurbo::Point,
) -> Option<(f64, f64)> {
    let s0 = settle_end(c, t0, p0, 0.0, t3);
    let s3 = settle_end(c, t3, p3, s0, 1.0);
    (s0 < s3).then_some((s0, s3))
}

/// The increasing map of heights that takes `from0` to `to0` and `from1` to `to1`, or
/// `None` when either range is empty or the map is not finite.
fn height_map(from0: f64, from1: f64, to0: f64, to1: f64) -> Option<impl Fn(f64) -> f64> {
    let scale = (to1 - to0) / (from1 - from0);
    (from0 < from1 && to0 < to1 && scale.is_finite()).then_some(move |y| to0 + (y - from0) * scale)
}

/// Cuts `c` between the points nearest `p0` and `p3`, searching from `t0` and `t3`, and
/// moves the piece onto `p0` and `p3`: each end moves sideways together with its handle,
/// and the heights are mapped linearly, so a curve that is monotonic in `y` stays so.
/// Returns `None` when the two cut points are not in order.
fn settled_cubic(
    c: CubicBez,
    t0: f64,
    t3: f64,
    p0: kurbo::Point,
    p3: kurbo::Point,
) -> Option<CubicBez> {
    let (s0, s3) = settled_range(c, t0, t3, p0, p3)?;
    let mut piece = c.subsegment(s0..s3);
    let fit = height_map(piece.p0.y, piece.p3.y, p0.y, p3.y)?;
    piece.p1.x += p0.x - piece.p0.x;
    piece.p2.x += p3.x - piece.p3.x;
    piece.p1.y = fit(piece.p1.y);
    piece.p2.y = fit(piece.p2.y);
    piece.p0 = p0;
    piece.p3 = p3;
    Some(piece)
}

/// Cuts `q` between the points nearest `p0` and `p2`, searching from `t0` and `t2`, and
/// moves the piece onto `p0` and `p2`: its control point moves sideways by the mean of the
/// two ends' moves, and the heights are mapped linearly, so a curve that is monotonic in
/// `y` stays so. Returns `None` when the two cut points are not in order.
fn settled_quad(
    q: QuadBez,
    t0: f64,
    t2: f64,
    p0: kurbo::Point,
    p2: kurbo::Point,
) -> Option<QuadBez> {
    // A raised quadratic keeps its parameters.
    let (s0, s2) = settled_range(q.raise(), t0, t2, p0, p2)?;
    let mut piece = q.subsegment(s0..s2);
    let fit = height_map(piece.p0.y, piece.p2.y, p0.y, p2.y)?;
    piece.p1.x += 0.5 * ((p0.x - piece.p0.x) + (p2.x - piece.p2.x));
    piece.p1.y = fit(piece.p1.y);
    piece.p0 = p0;
    piece.p2 = p2;
    Some(piece)
}

/// A positioned output segment, possibly perturbed from the original.
///
/// We perturb output segments in order to strictly enforce ordering.
/// A single segment might be expanded into a longer path.
#[derive(Default, Debug)]
pub struct PositionedOutputSeg {
    /// The path, which is guaranteed to start and end at the same points as the
    /// output segment did.
    pub path: BezPath,
    /// Sometimes (often, even) a large part of the output segment didn't need
    /// to be perturbed. Our sweep-line invariants guarantee that unperturbed
    /// part can only be a single contiguous interval: this stores the index
    /// of the unperturbed part within `path.segments()`.
    pub copied_idx: Option<usize>,
}

impl PositionedOutputSeg {
    /// Creates an output segment from a kurbo segment, truncated and perturbed at the endpoints.
    pub fn from_seg(s: kurbo::PathSeg, start: kurbo::Point, end: kurbo::Point) -> Self {
        if start.y == end.y {
            // Horizontal segments are always straight.
            PositionedOutputSeg::from_line((start, end))
        } else {
            match s {
                kurbo::PathSeg::Line(_) => PositionedOutputSeg::from_line((start, end)),
                kurbo::PathSeg::Quad(q) => PositionedOutputSeg::from_quad(q, start, end),
                kurbo::PathSeg::Cubic(c) => PositionedOutputSeg::from_cubic(c, start, end),
            }
        }
    }

    pub fn from_line(ell: impl Into<kurbo::Line>) -> Self {
        let mut path = BezPath::with_capacity(2);
        let ell = ell.into();
        path.move_to(ell.p0);
        path.line_to(ell.p1);
        PositionedOutputSeg {
            path,
            copied_idx: Some(0),
        }
    }

    /// Creates an output segment from a cubic, truncated and perturbed at the endpoints.
    ///
    /// The cubic is assumed to be monotonic in `y`. We assume that `p0.y < p3.y`, and
    /// both must be in the `y`-range of the cubic. The cubic is truncated to be between
    /// the heights `p0.y` and `p3.y`, and then perturbed horizontally so that it starts
    /// exactly and at `p0` and ends exactly at `p3`.
    ///
    /// Craft patch (flat-end shear): when `p0` or `p3` is off the curve, the cubic is cut
    /// at the points nearest `p0` and `p3` instead, each end of the piece moves sideways
    /// with its handle, and its heights are mapped linearly onto `p0.y..p3.y`.
    pub fn from_cubic(c: kurbo::CubicBez, p0: kurbo::Point, p3: kurbo::Point) -> Self {
        debug_assert!(p0.y >= c.p0.y);
        debug_assert!(p3.y <= c.p3.y);
        debug_assert!(p0.y < p3.y);

        let t0 = solve_t_for_y(c, p0.y);
        let t3 = solve_t_for_y(c, p3.y);
        let orig = c;
        let mut c = c.subsegment(t0..t3);

        let dx0 = p0.x - c.p0.x;
        let dx3 = p3.x - c.p3.x;

        // Craft patch (flat-end shear)
        if (dx0 != 0.0 || dx3 != 0.0)
            && let Some(piece) = settled_cubic(orig, t0, t3, p0, p3)
        {
            let mut path = BezPath::with_capacity(2);
            path.move_to(p0);
            path.curve_to(piece.p1, piece.p2, p3);
            return PositionedOutputSeg {
                path,
                copied_idx: Some(0),
            };
        }

        let s1 = ((c.p1.y - p0.y) / (p3.y - p0.y)).clamp(0.0, 1.0);
        let s2 = ((c.p2.y - p0.y) / (p3.y - p0.y)).clamp(0.0, 1.0);

        c.p1.x += (1.0 - s1) * dx0 + s1 * dx3;
        c.p2.x += (1.0 - s2) * dx0 + s2 * dx3;

        let mut path = BezPath::with_capacity(2);
        path.move_to(p0);
        path.curve_to(c.p1, c.p2, p3);
        PositionedOutputSeg {
            path,
            copied_idx: Some(0),
        }
    }

    /// Creates an output segment from a quadratic, truncated and perturbed at the endpoints.
    ///
    /// The cubic is assumed to be monotonic in `y`. We assume that `p0.y < p2.y`, and
    /// both must be in the `y`-range of the cubic. The cubic is truncated to be between
    /// the heights `p0.y` and `p2.y`, and then perturbed horizontally so that it starts
    /// exactly and at `p0` and ends exactly at `p2`.
    ///
    /// Craft patch (flat-end shear): when `p0` or `p2` is off the curve, the quadratic is
    /// cut at the points nearest `p0` and `p2` instead, its control point moves sideways
    /// by the mean of the two ends' moves, and its heights are mapped linearly onto
    /// `p0.y..p2.y`.
    pub fn from_quad(q: kurbo::QuadBez, p0: kurbo::Point, p2: kurbo::Point) -> Self {
        debug_assert!(p0.y >= q.p0.y);
        debug_assert!(p2.y <= q.p2.y);
        debug_assert!(p0.y < p2.y);

        fn solve_t_for_y(q: kurbo::QuadBez, y: f64) -> f64 {
            // Write the y coordinate of `q - y` as `at^2 + bt + c`.
            let c = q.p0.y - y;
            let b = 2.0 * (q.p1.y - q.p0.y);
            let a = q.p0.y - 2.0 * q.p1.y + q.p2.y;
            let roots = Quadratic::new([c, b, a]).roots();
            for r in roots {
                if (0.0..=1.0).contains(&r) {
                    return r;
                }
            }
            // If we land here, y should be close to one of the endpoints and it
            // was just numerical error that pushed the root out of [0.0, 1.0].
            // The only question is which endpoint.
            if y >= 0.5 * (q.p0.y + q.p2.y) {
                1.0
            } else {
                0.0
            }
        }
        let t0 = solve_t_for_y(q, p0.y);
        let t2 = solve_t_for_y(q, p2.y);
        let orig = q;
        let mut q = q.subsegment(t0..t2);

        let dx0 = p0.x - q.p0.x;
        let dx2 = p2.x - q.p2.x;

        // Craft patch (flat-end shear)
        if (dx0 != 0.0 || dx2 != 0.0)
            && let Some(piece) = settled_quad(orig, t0, t2, p0, p2)
        {
            let mut path = BezPath::with_capacity(2);
            path.move_to(p0);
            path.quad_to(piece.p1, p2);
            return PositionedOutputSeg {
                path,
                copied_idx: Some(0),
            };
        }

        let s = ((q.p1.y - p0.y) / (p2.y - p0.y)).clamp(0.0, 1.0);

        q.p1.x += (1.0 - s) * dx0 + s * dx2;

        let mut path = BezPath::with_capacity(2);
        path.move_to(p0);
        path.quad_to(q.p1, p2);
        PositionedOutputSeg {
            path,
            copied_idx: Some(0),
        }
    }
}

/// Compute positions for all of the output segments.
///
/// The orders between the output segments is specified by `order`. The endpoints
/// should have been already computed (in a way that satisfies the order), and
/// are provided in `endpoints`. For each output segment, we return a Bézier
/// path.
pub(crate) fn compute_positions(
    segs: &Segments,
    orig_seg_map: &OutputSegVec<SegIdx>,
    cmp: &mut ComparisonCache,
    endpoints: &HalfOutputSegVec<kurbo::Point>,
    scan_order: &ScanLineOrder,
    accuracy: f64,
) -> OutputSegVec<PositionedOutputSeg> {
    let mut out = OutputSegVec::<PositionedOutputSeg>::with_size(orig_seg_map.len());
    // We try to build `out` lazily, by avoiding copying input segments to outputs segments
    // until they're needed (by copying in one go, we avoid excess subdivisions). That means
    // we need to separately keep track of how far down we've looked at each output. If
    // we weren't lazy, that would just be the last `y` coordinate in `out`.
    let mut last_y = OutputSegVec::<f64>::with_size(orig_seg_map.len());
    let mut queue = BinaryHeap::<HeapEntry>::new();
    for idx in out.indices() {
        let p = endpoints[idx.first_half()];
        let q = endpoints[idx.second_half()];
        if p.y == q.y {
            out[idx].path.move_to(p);
            out[idx].path.line_to(q);
            out[idx].copied_idx = Some(0);
            continue;
        }
        out[idx].path.move_to(p);
        queue.push(HeapEntry { y: p.y.into(), idx });
    }

    while let Some(entry) = queue.pop() {
        // Maybe this entry was already handled by one of its neighbors, in which case we skip it.
        let y0 = entry.y.into_inner();
        if last_y[entry.idx] > y0 {
            continue;
        }

        let mut y1 = endpoints[entry.idx.second_half()].y;
        let mut west_scan = vec![entry.idx];
        let mut cur = entry.idx;
        while let Some(nbr) = scan_order.west_neighbor_after(cur, y0) {
            let order = cmp.compare_segments(segs, orig_seg_map[nbr], orig_seg_map[cur]);
            let (_, cmp_end_y, cmp_order) = order.iter().find(|(_, end_y, _)| *end_y > y0).unwrap();

            if cmp_order == Order::Left {
                let next_close_y = scan_order
                    .close_west_neighbor_height_after(cur, y0, orig_seg_map, segs, cmp)
                    .unwrap_or(f64::INFINITY);
                y1 = y1.min(next_close_y.max(cmp_end_y));
                break;
            } else {
                y1 = y1.min(cmp_end_y).min(endpoints[nbr.second_half()].y);
                west_scan.push(nbr);
            }
            cur = nbr;
        }

        let mut east_scan = vec![];
        let mut cur = entry.idx;
        while let Some(nbr) = scan_order.east_neighbor_after(cur, y0) {
            let order = cmp.compare_segments(segs, orig_seg_map[cur], orig_seg_map[nbr]);
            let (_, cmp_end_y, cmp_order) = order.iter().find(|(_, end_y, _)| *end_y > y0).unwrap();

            if cmp_order == Order::Left {
                let next_close_y = scan_order
                    .close_east_neighbor_height_after(cur, y0, orig_seg_map, segs, cmp)
                    .unwrap_or(f64::INFINITY);
                y1 = y1.min(next_close_y.max(cmp_end_y));
                break;
            } else {
                y1 = y1.min(cmp_end_y).min(endpoints[nbr.second_half()].y);
                east_scan.push(nbr);
            }
            cur = nbr;
        }

        let mut neighbors = west_scan;
        neighbors.reverse();
        neighbors.extend(east_scan);
        if neighbors.len() == 1 {
            let idx = entry.idx;
            // We're far from everything, so we can just copy the input bezier to the output.
            // We only actually do this eagerly for horizontal segments: for other segments
            // we'll hold off on the copying because it might allow us to avoid further
            // subdivision.
            if y0 == y1 {
                out[idx].copied_idx = Some(out[idx].path.elements().len() - 1);
                out[idx].path.line_to(endpoints[idx.second_half()]);
            }
        } else {
            let orig_neighbors = neighbors
                .iter()
                .map(|idx| orig_seg_map[*idx])
                .collect::<Vec<_>>();

            ordered_curves_all_close(
                segs,
                &orig_neighbors,
                &neighbors,
                &mut out,
                y0,
                y1,
                endpoints,
                accuracy,
            );
        }
        for idx in neighbors {
            let y_end = endpoints[idx.second_half()].y;
            if y1 < y_end {
                queue.push(HeapEntry { y: y1.into(), idx });
            }
            last_y[idx] = y1;
        }
    }

    for out_idx in out.indices() {
        let out = &mut out[out_idx];
        let endpoint = endpoints[out_idx.second_half()];
        if bez_end_y(&out.path) < endpoint.y {
            let seg_idx = orig_seg_map[out_idx];
            out.copied_idx = Some(out.path.elements().len() - 1);
            if segs[seg_idx].is_line() {
                out.path.line_to(endpoint)
            } else {
                let c = next_subsegment(&segs[seg_idx], &out.path, endpoint.y, endpoint);
                out.path.curve_to(c.p1, c.p2, c.p3);
            }
        } else {
            // The quadratic approximations don't respect the fixed endpoints, so tidy them
            // up. Since both the quadratic approximations and the endpoints satisfy
            // the ordering, this doesn't mess up the ordering.
            match out.path.elements_mut().last_mut().unwrap() {
                kurbo::PathEl::MoveTo(p)
                | kurbo::PathEl::LineTo(p)
                | kurbo::PathEl::QuadTo(_, p)
                | kurbo::PathEl::CurveTo(_, _, p) => {
                    *p = endpoints[out_idx.second_half()];
                }
                kurbo::PathEl::ClosePath => unreachable!(),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use kurbo::{
        Affine, BezPath, CubicBez, Line, ParamCurve, ParamCurveNearest, PathSeg, Point, QuadBez,
        Shape,
    };

    use crate::Segments;

    use super::{PositionedOutputSeg, horizontal_error_weight};

    #[test]
    fn error_weight() {
        // Vertical lines have a weight of 1.
        let c: CubicBez = PathSeg::from(Line::new((0.0, 0.0), (0.0, 1.0))).to_cubic();
        assert_eq!(horizontal_error_weight(c), 1.0);

        // Horizontal lines have a weight of 0.
        let c: CubicBez = PathSeg::from(Line::new((1.0, 0.0), (0.0, 0.0))).to_cubic();
        assert_eq!(horizontal_error_weight(c), 0.0);

        let c: CubicBez = PathSeg::from(Line::new((0.0, 0.0), (1.0, 0.0))).to_cubic();
        assert_eq!(horizontal_error_weight(c), 0.0);

        // S-shaped curves have a weight of 1
        let c = CubicBez::new((0.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0));
        assert_eq!(horizontal_error_weight(c), 1.0);

        // Diagonal lines have a weight of about 1/sqrt(2).
        let c: CubicBez = PathSeg::from(Line::new((0.0, 0.0), (1.0, 1.0))).to_cubic();
        assert_eq!(horizontal_error_weight(c), 1.0 / 2.0f64.sqrt());
    }

    // Here's an example of lines where we used to have a bad
    // approximation. Check that they only require a single
    // approximation step.
    #[test]
    fn linear_approx() {
        let mut p0 = BezPath::new();
        p0.move_to((-212.00062561035156, 90.03582000732422));
        p0.line_to((211.99937438964844, 90.03646087646484));
        p0.close_path();

        let mut p1 = BezPath::new();
        p1.move_to((211.99964904785153, -90.0358200073242));
        p1.line_to((211.99937438964844, 90.03646087646484));
        p1.close_path();

        let mut segs = Segments::default();
        segs.add_bez_path(&p0).unwrap();
        segs.add_bez_path(&p1).unwrap();
        let s0 = segs.indices().next().unwrap();
        let s1 = segs.indices().nth(2).unwrap();

        let mut out = Vec::new();
        let y1 = 90.03646087646484;
        let next_y0 = super::approximate(90.03645987646233, y1, &[s0, s1], &segs, 2.5e-7, &mut out);
        assert_eq!(next_y0, y1);
    }

    // The largest distance from a point of `path` to the nearest of `inputs`.
    fn farthest_from(path: &BezPath, inputs: &[PathSeg]) -> f64 {
        let mut worst = 0.0f64;
        for seg in path.segments() {
            for k in 0..=200 {
                let p = seg.eval(k as f64 / 200.0);
                let d = inputs
                    .iter()
                    .map(|c| c.nearest(p, 1e-12).distance_sq.sqrt())
                    .fold(f64::INFINITY, f64::min);
                worst = worst.max(d);
            }
        }
        worst
    }

    // Places `seg` between `p0` and `p1` and checks that the result runs from `p0`
    // to `p1` and stays within `tol` of `seg`.
    fn assert_placed_near(seg: PathSeg, p0: Point, p1: Point, tol: f64) {
        let out = PositionedOutputSeg::from_seg(seg, p0, p1);
        let same_kind = |s: PathSeg| std::mem::discriminant(&s) == std::mem::discriminant(&seg);
        assert!(out.path.segments().all(same_kind));
        assert_eq!(out.path.segments().next().unwrap().start(), p0);
        assert_eq!(out.path.segments().last().unwrap().end(), p1);
        let d = farthest_from(&out.path, &[seg]);
        assert!(d < tol, "{seg:?} from {p0:?} to {p1:?}: {d}");
    }

    // Craft patch (flat-end shear): a curve with a repeated handle at its flat
    // end has zero derivative there, so a Newton step from that end cannot
    // leave it. The end point is moved along the flat part.
    #[test]
    fn repeated_end_handle_stays_near_the_end() {
        let origin = Point::new(0.0, 0.0);
        let top_right = Point::new(1000.0, 1000.0);
        let c = CubicBez::new(origin, Point::new(0.0, 1000.0), top_right, top_right);
        // Moved along the flat part, the end point is 0.0127 from the curve.
        let end = Point::new(998.37, 1000.0);
        assert_placed_near(PathSeg::Cubic(c), origin, end, 0.02);

        let start = Point::new(1.63, 0.0);
        let corner = Point::new(1000.0, 0.0);
        let c = CubicBez::new(origin, origin, corner, top_right);
        assert_placed_near(PathSeg::Cubic(c), start, top_right, 0.02);
    }

    // Craft patch (flat-end shear): a curve that is almost flat at one end and
    // steep at the other, with the flat end's point moved along the flat part,
    // keeps its steep part in place.
    #[test]
    fn flat_end_shift_stays_near_the_end() {
        // Steep start, flat end, as cubic and quadratic.
        let origin = Point::new(0.0, 0.0);
        let (top_left, top_right) = (Point::new(0.0, 1000.0), Point::new(1000.0, 1000.0));
        let c = CubicBez::new(origin, top_left, Point::new(300.0, 1000.0), top_right);
        let end = Point::new(998.37, 1000.0);
        assert_placed_near(PathSeg::Cubic(c), origin, end, 1e-5);
        let q = QuadBez::new(origin, top_left, top_right);
        let end = Point::new(999.95, 1000.0);
        assert_placed_near(PathSeg::Quad(q), origin, end, 1e-5);

        // Flat start, steep end.
        let corner = Point::new(1000.0, 0.0);
        let c = CubicBez::new(origin, Point::new(700.0, 0.0), corner, top_right);
        assert_placed_near(PathSeg::Cubic(c), Point::new(1.63, 0.0), top_right, 1e-5);
        let q = QuadBez::new(origin, corner, top_right);
        assert_placed_near(PathSeg::Quad(q), Point::new(0.05, 0.0), top_right, 1e-5);

        // The end point just below the flat end, as the sweep places it: the cut
        // from `solve_t_for_y` lies 0.75 to the left of it.
        let c = CubicBez::new(origin, top_left, Point::new(300.0, 1000.0), top_right);
        let end = Point::new(999.99999674, 999.9999999534);
        assert_placed_near(PathSeg::Cubic(c), origin, end, 1e-6);
    }

    // Craft patch (flat-end shear): a piece placed between end points off its
    // curve stays monotonic in `y`.
    #[test]
    fn flat_end_pieces_stay_monotonic() {
        let (p0, p1, p2) = (
            Point::new(0.0, 0.0),
            Point::new(100.0, 200.0),
            Point::new(200.0, 200.0),
        );
        let end = Point::new(200.0, 199.0);
        let segs = [
            PathSeg::Quad(QuadBez::new(p0, p1, p2)),
            PathSeg::Cubic(QuadBez::new(p0, p1, p2).raise()),
        ];
        for seg in segs {
            let out = PositionedOutputSeg::from_seg(seg, p0, end);
            for piece in out.path.segments() {
                let mut last = piece.start().y;
                for k in 1..=200 {
                    let y = piece.eval(k as f64 / 200.0).y;
                    assert!(y >= last && y <= end.y, "{seg:?}: {y} after {last}");
                    last = y;
                }
            }
        }
    }

    // Craft patch (flat-end shear): the union of a shape with a copy moved by
    // less than `eps` stays on the two shapes, and keeps their area.
    #[test]
    fn flat_end_union_stays_on_the_inputs() {
        let svg = "M0,0 C0,1000 300,1000 1000,1000 L1000,2000 L-1000,2000 Z";
        let a = BezPath::from_svg(svg).unwrap();
        let mut b = a.clone();
        b.apply_affine(Affine::translate((-3.26e-6, -4.66e-8)));
        let inputs: Vec<PathSeg> = a.segments().chain(b.segments()).collect();
        let (fill, op) = (crate::FillRule::NonZero, crate::BinaryOp::Union);
        let out = crate::binary_op_with_eps(&a, &b, fill, op, 1e-6).unwrap();
        let mut area = 0.0;
        for contour in out.contours() {
            area += contour.path.area().abs();
            let d = farthest_from(&contour.path, &inputs);
            assert!(d < 1e-5, "{d}");
        }
        assert!((area - 2_095_000.006_5).abs() < 1e-3, "{area}");
    }

    // Craft patch (flat-end shear): a rectangle whose left side crosses the flat
    // end of a curve, away from its anchors, leaves the curve in place in its
    // union, with both layouts.
    #[test]
    fn flat_end_crossing_stays_on_the_inputs() {
        use crate::sweep::Correctness;
        use crate::topology::{BinaryWindingNumber, OneOfTwo, Topology};

        let svg = "M0,0 C0,1000 300,1000 1000,1000 L1000,2000 L-1000,2000 Z";
        let a = BezPath::from_svg(svg).unwrap();
        for xv in [998.37, 990.37] {
            let rect = format!("M{xv},500 L1500,500 L1500,1500 L{xv},1500 Z");
            let b = BezPath::from_svg(&rect).unwrap();
            let inputs: Vec<PathSeg> = a.segments().chain(b.segments()).collect();
            for correctness in [Correctness::GoodEnough, Correctness::ReallyCorrect] {
                let paths = [(&a, OneOfTwo::A), (&b, OneOfTwo::B)];
                let top = Topology::<BinaryWindingNumber>::from_paths_with_correctness(
                    paths,
                    1e-6,
                    correctness,
                )
                .unwrap();
                let out = top.contours(|w| w.shape_a != 0 || w.shape_b != 0);
                let mut area = 0.0;
                for contour in out.contours() {
                    area += contour.path.area().abs();
                    let d = farthest_from(&contour.path, &inputs);
                    assert!(d < 1e-5, "{xv} {correctness:?}: {d}");
                }
                let expected = 2_595_000.0 + (1000.0 - xv) * 500.0;
                assert!(
                    (area - expected).abs() < 1e-2,
                    "{xv} {correctness:?}: {area}"
                );
            }
        }
    }
}
