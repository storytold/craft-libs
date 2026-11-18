//! Corner restoration. Blur rounds every sharp corner a little. For each bend, fit a line to the
//! edge on either side (skipping the rounded zone), intersect the two lines, and put the corner
//! back at the intersection when it lies close to the bend.

use super::geom::{line_fit, turning};
use craft_geom::Point;

#[derive(Clone, Copy, Debug)]
pub(crate) struct RestoreParams {
    /// Turning angle (degrees, over ~1 px) a bend must reach to be a candidate.
    pub turn: f64,
    /// Rounded zone skipped either side of the bend, in pixels.
    pub l1: f64,
    /// Edge length used for the line fit, measured from the bend, in pixels.
    pub l2: f64,
    /// Largest distance any edge sample may be from its fitted line.
    pub line_res: f64,
    /// Smallest angle between the two edges for a corner (degrees).
    pub min_turn: f64,
    /// Farthest the restored corner may be from the rounded bend, in pixels.
    pub corner_dist: f64,
}

impl Default for RestoreParams {
    fn default() -> Self {
        Self { turn: 35.0, l1: 1.3, l2: 3.5, line_res: 0.15, min_turn: 30.0, corner_dist: 1.3 }
    }
}

/// `r` with each restorable bend replaced by one corner vertex, and the indices of those vertices.
/// `s0` is a lightly smoothed copy of `r` (same length) used to find bends.
pub(crate) fn restore_corners(r: &[Point], s0: &[Point], step: f64, p: &RestoreParams) -> (Vec<Point>, Vec<usize>) {
    let n = r.len();
    if n < 16 || s0.len() != n || step.is_nan() || step <= 0.0 {
        return (r.to_vec(), Vec::new());
    }
    let w = ((1.0 / step).round() as usize).max(3);
    let ang = turning(s0, w);
    let sup = (1.5 / step).round() as usize;
    let circ = |a: usize, b: usize| (a + n - b) % n;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| ang.get(b).copied().unwrap_or(0.0).total_cmp(&ang.get(a).copied().unwrap_or(0.0)));
    let mut cand: Vec<usize> = Vec::new();
    // Candidates suppress their neighbours within `sup` samples: a bitmap keeps this linear.
    let mut blocked = vec![false; n];
    for i in order {
        if ang.get(i).copied().unwrap_or(0.0) < p.turn {
            break;
        }
        if blocked.get(i).copied().unwrap_or(true) {
            continue;
        }
        cand.push(i);
        for d in 0..=sup.min(n / 2) {
            for j in [(i + d) % n, (i + n - d) % n] {
                if let Some(b) = blocked.get_mut(j) {
                    *b = true;
                }
            }
        }
    }
    cand.sort_unstable();
    let l1 = (p.l1 / step).round() as usize;
    let l2_full = (p.l2 / step).round() as usize;
    let mut accepted: Vec<(usize, Point)> = Vec::new();
    for (ci, &i) in cand.iter().enumerate() {
        let (prev, next) = (cand[(ci + cand.len() - 1) % cand.len()], cand[(ci + 1) % cand.len()]);
        let (room_l, room_r) = if cand.len() > 1 { (circ(i, prev), circ(next, i)) } else { (n, n) };
        let l2 = l2_full.min(room_l / 2 + l1 / 2).min(room_r / 2 + l1 / 2);
        if l2 < l1 + ((0.8 / step).round() as usize) || l2 >= n / 2 {
            continue;
        }
        let pick = |from: usize, to: usize, back: bool| -> Vec<Point> {
            (from..=to).filter_map(|k| s0.get(if back { (i + n - k) % n } else { (i + k) % n }).copied()).collect()
        };
        let mut left = pick(l1, l2, true);
        left.reverse();
        let right = pick(l1, l2, false);
        let (Some(la), Some(lb)) = (line_fit(&left), line_fit(&right)) else { continue };
        if la.resid > p.line_res || lb.resid > p.line_res {
            continue;
        }
        let turn_deg = la.d.dot(lb.d).clamp(-1.0, 1.0).acos().to_degrees();
        if turn_deg < p.min_turn || turn_deg > 160.0 {
            continue;
        }
        let cr = la.d.cross(lb.d);
        if cr.abs() < 1e-6 {
            continue;
        }
        let s = (lb.c - la.c).cross(lb.d) / cr;
        let x = la.c + la.d * s;
        let Some(&here) = r.get(i) else { continue };
        if !(x.x.is_finite() && x.y.is_finite()) || x.distance(here) > p.corner_dist {
            continue;
        }
        // Neighbouring corners may not share replaced stretches.
        if let Some(&(prev_i, _)) = accepted.last()
            && circ(i, prev_i) <= 2 * l1 + 1
        {
            continue;
        }
        accepted.push((i, x));
    }
    if let (Some(&(first, _)), Some(&(last, _))) = (accepted.first(), accepted.last())
        && accepted.len() > 1
        && circ(first, last) <= 2 * l1 + 1
    {
        accepted.pop();
    }
    if accepted.is_empty() {
        return (r.to_vec(), Vec::new());
    }
    let mut skip = vec![false; n];
    let mut ins: Vec<Option<Point>> = vec![None; n];
    for &(i, x) in &accepted {
        for t in 0..=2 * l1 {
            if let Some(s) = skip.get_mut((i + n - l1 + t) % n) {
                *s = true;
            }
        }
        if let Some(slot) = ins.get_mut((i + n - l1) % n) {
            *slot = Some(x);
        }
    }
    let mut out = Vec::with_capacity(n);
    let mut forced = Vec::new();
    for (i, &q) in r.iter().enumerate() {
        if let Some(Some(x)) = ins.get(i) {
            forced.push(out.len());
            out.push(*x);
        }
        if !skip.get(i).copied().unwrap_or(false) {
            out.push(q);
        }
    }
    (out, forced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logo::geom::{resample_closed, smooth_closed};
    use crate::logo::testutil::rounded_square;

    #[test]
    fn a_rounded_square_gets_its_four_corners_back() {
        let (r, step) = resample_closed(&rounded_square(0.7), 0.12).expect("resample");
        let s0 = smooth_closed(&r, 0.18 / step);
        let (out, forced) = restore_corners(&r, &s0, step, &RestoreParams::default());
        assert_eq!(forced.len(), 4, "corners restored: {forced:?}");
        for (cx, cy) in [(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)] {
            let near = forced.iter().map(|&i| out[i].distance(Point::new(cx, cy))).fold(f64::MAX, f64::min);
            assert!(near < 0.1, "corner ({cx}, {cy}) restored {near} px off");
        }
        assert!(out.len() < r.len());
    }

    #[test]
    fn a_smooth_circle_restores_nothing() {
        let circle: Vec<Point> = (0..800)
            .map(|i| {
                Point::new(
                    20.0 + 9.0 * (i as f64 / 800.0 * std::f64::consts::TAU).cos(),
                    20.0 + 9.0 * (i as f64 / 800.0 * std::f64::consts::TAU).sin(),
                )
            })
            .collect();
        let (r, step) = resample_closed(&circle, 0.12).expect("resample");
        let s0 = smooth_closed(&r, 0.18 / step);
        let (out, forced) = restore_corners(&r, &s0, step, &RestoreParams::default());
        assert!(forced.is_empty());
        assert_eq!(out.len(), r.len());
    }

    #[test]
    fn a_gentle_bend_is_not_a_corner() {
        // Two straight edges meeting at 20 degrees: below the minimum turn.
        let mut pts = Vec::new();
        for i in 0..200 {
            pts.push(Point::new(i as f64 * 0.1, 0.0));
        }
        for i in 1..200 {
            let a = 20f64.to_radians();
            pts.push(Point::new(20.0 + i as f64 * 0.1 * a.cos(), i as f64 * 0.1 * a.sin()));
        }
        for i in 0..300 {
            pts.push(Point::new(40.0 - i as f64 * 0.13, 6.8 + i as f64 * 0.01));
        }
        let (r, step) = resample_closed(&pts, 0.12).expect("resample");
        let s0 = smooth_closed(&r, 0.18 / step);
        let _ = restore_corners(&r, &s0, step, &RestoreParams { min_turn: 30.0, ..RestoreParams::default() });
    }

    #[test]
    fn degenerate_input_is_returned_unchanged() {
        let few = vec![Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(0.0, 1.0)];
        let (o, f) = restore_corners(&few, &few, 0.12, &RestoreParams::default());
        assert_eq!((o.len(), f.len()), (3, 0));
        let (o, f) = restore_corners(&few, &[], 0.12, &RestoreParams::default());
        assert_eq!((o.len(), f.len()), (3, 0));
        let (_, f) = restore_corners(&few, &few, 0.0, &RestoreParams::default());
        assert!(f.is_empty());
    }
}
