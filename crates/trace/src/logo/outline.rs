//! One outline: smooth, restore corners, snap to lines and arcs, fit Béziers, and verify.

use super::bezier::{fit_cubics, tangent};
use super::geom::{
    Cubic, all_within, arc_cubics, circle_fit, find_corners, finite, resample_closed, resample_open, sample_cubics, smooth_closed, smooth_open,
    smooth_pinned,
};
use super::restore::{RestoreParams, restore_corners};
use craft_geom::Point;

/// How an outline is fitted. Lengths are in (working) pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    pub step: f64,
    pub sigma: f64,
    pub corner_deg: f64,
    pub corner_win: f64,
    pub tol: f64,
    pub snap: bool,
    /// With `snap`: parts of circles become true arcs too.
    pub arcs: bool,
    pub line_tol: f64,
    pub circle_tol: f64,
    pub restore: Option<RestoreParams>,
    /// An outline whose fit strays farther than this from the raw contour is fitted plainly.
    pub max_dev: f64,
    /// Spacing of the points of the fit that this check looks at.
    pub check_step: f64,
    /// Straight pieces within a few degrees of level or upright are made exactly so (by the
    /// tracer's other modes; Flat Logo leaves them).
    pub upright: bool,
    /// The outline is the contour of a smooth area: a run between corners is smoothed by `sigma`
    /// whatever its length (not by at most an eighth of it), and the fit is checked against the
    /// outline smoothed by `sigma` (not against the outline itself).
    pub contour: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            step: 0.12,
            sigma: 0.5,
            corner_deg: 60.0,
            corner_win: 0.7,
            tol: 0.12,
            snap: true,
            arcs: true,
            line_tol: 0.22,
            circle_tol: 0.25,
            restore: Some(RestoreParams::default()),
            max_dev: 1.4,
            check_step: 0.03,
            upright: false,
            contour: false,
        }
    }
}

/// What fitting did, for tests and diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub restored: usize,
    pub lines: usize,
    pub arcs: usize,
    pub circles: usize,
    pub reverted: usize,
}

fn line_cubic(a: Point, b: Point) -> Cubic {
    let d = b - a;
    [a, a + d / 3.0, a + d * (2.0 / 3.0), b]
}

/// The sweep of angles from the first to the last point of `pts` about `c` (continuous, may exceed π).
fn sweep(pts: &[Point], c: Point) -> Option<(f64, f64)> {
    let ang = |p: &Point| (p.y - c.y).atan2(p.x - c.x);
    let mut prev = ang(pts.first()?);
    let a0 = prev;
    let mut total = 0.0;
    for p in pts.iter().skip(1) {
        let a = ang(p);
        let mut d = a - prev;
        while d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        }
        while d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        total += d;
        prev = a;
    }
    Some((a0, a0 + total))
}

/// A plain fit of the closed raw contour: no smoothing, corners or snapping.
fn plain_fit(p: &[Point], tol: f64) -> Vec<Cubic> {
    let n = p.len();
    let (Some(&first), Some(&second), Some(&last)) = (p.first(), p.get(1), p.get(n.saturating_sub(1))) else { return Vec::new() };
    let t = second - last;
    let len = t.hypot();
    if len.is_nan() || len <= 1e-12 {
        return Vec::new();
    }
    let t = t / len;
    let mut closed = p.to_vec();
    closed.push(first);
    fit_cubics(&closed, t, -t, tol)
}

/// One run between two corners, added to `segs`: a line or an arc where it is one, else fitted
/// curves. `sg` is the smoothing, in samples.
fn fit_run(run: &[Point], sg: f64, st: &Style, stats: &mut Stats, segs: &mut Vec<Cubic>) {
    let (Some(&first), Some(&last)) = (run.first(), run.last()) else { return };
    if run.len() < 4 {
        if run.len() >= 2 && first.distance(last) > 1e-9 {
            segs.push(line_cubic(first, last));
        }
        return;
    }
    let sm = if st.contour { smooth_pinned(run, sg) } else { smooth_open(run, sg) };
    let (Some(&a), Some(&b)) = (sm.first(), sm.last()) else { return };
    let len = a.distance(b);
    if st.snap && len > 1.5 {
        let d = b - a;
        let nrm = kurbo::Vec2::new(-d.y, d.x) / len;
        if sm.iter().all(|q| (*q - a).dot(nrm).abs() < st.line_tol) {
            stats.lines += 1;
            segs.push(line_cubic(a, b));
            return;
        }
    }
    if st.snap
        && st.arcs
        && sm.len() > 12
        && len > 1.0
        && let Some(c) = circle_fit(&sm)
        && c.resid < st.circle_tol
        && c.r > 2.5
        && c.r < 400.0
        && let Some((a0, a1)) = sweep(&sm, c.c)
        && (a1 - a0).abs() > 0.35
    {
        stats.arcs += 1;
        let first = segs.len();
        segs.extend(arc_cubics(c.c, c.r, a0, a1));
        // The fitted circle may miss the corners by up to the snap tolerance: end exactly on them.
        if let Some(s) = segs.get_mut(first) {
            s[0] = a;
        }
        if let Some(s) = segs.last_mut() {
            s[3] = b;
        }
        return;
    }
    segs.extend(fit_cubics(&sm, tangent(&sm, 0, 4, 1), tangent(&sm, sm.len() - 1, 4, -1), st.tol));
}

/// The open run `p` as cubic Béziers from its first point to its last, both kept exactly: cut at
/// its corners, each stretch a line, an arc or fitted curves as in a closed outline. (No corner is
/// put back here: a piece of an outline between two junctions ends on pixel corners anyway.)
pub(crate) fn fit_open(p: &[Point], st: &Style, stats: &mut Stats) -> Vec<Cubic> {
    let mut segs = Vec::new();
    let (Some(&first), Some(&last)) = (p.first(), p.last()) else { return segs };
    let whole = |segs: &mut Vec<Cubic>| {
        if first.distance(last) > 1e-9 {
            segs.push(line_cubic(first, last));
        }
    };
    let Some((r, step)) = resample_open(p, st.step) else {
        whole(&mut segs);
        return segs;
    };
    let n = r.len();
    let s0 = smooth_open(&r, (0.18 / step).max(1.0));
    let w = ((st.corner_win / step).round() as usize).max(2);
    let angle = |i: usize| {
        let (Some(&c), Some(&a), Some(&b)) = (s0.get(i), i.checked_sub(w).and_then(|j| s0.get(j)), s0.get(i + w)) else { return 0.0 };
        let (v1, v2) = (c - a, b - c);
        v1.cross(v2).atan2(v1.dot(v2)).abs().to_degrees()
    };
    let mut cuts = vec![0];
    for i in w..n.saturating_sub(w) {
        let here = angle(i);
        if here >= st.corner_deg && here >= angle(i - 1) && here > angle(i + 1) && cuts.last().is_none_or(|&l| i - l > w) {
            cuts.push(i);
        }
    }
    cuts.push(n - 1);
    let sg = st.sigma / step;
    for pair in cuts.windows(2) {
        let Some(run) = r.get(pair[0]..=pair[1]) else { continue };
        let before = segs.len();
        fit_run(run, sg, st, stats, &mut segs);
        // Every stretch must lead on to the next.
        if segs.len() == before
            && let (Some(&a), Some(&b)) = (run.first(), run.last())
            && a.distance(b) > 1e-9
        {
            segs.push(line_cubic(a, b));
        }
    }
    if !sane(&segs) {
        segs.clear();
        whole(&mut segs);
    }
    segs
}

fn styled_fit(p: &[Point], st: &Style, stats: &mut Stats) -> Option<Vec<Cubic>> {
    let (mut r, step) = resample_closed(p, st.step)?;
    let sg = st.sigma / step;
    let light = (0.18 / step).max(1.0);
    let mut s0 = smooth_closed(&r, light);
    let mut forced: Vec<usize> = Vec::new();
    if let Some(rp) = &st.restore {
        let (nr, f) = restore_corners(&r, &s0, step, rp);
        if !f.is_empty() {
            stats.restored += f.len();
            s0 = nr.clone();
            r = nr;
            forced = f;
        }
    }
    let w = ((st.corner_win / step).round() as usize).max(2);
    let mut corners = find_corners(&s0, w, st.corner_deg);
    corners.extend(forced);
    corners.sort_unstable();
    corners.dedup();
    let n = r.len();
    if corners.is_empty() {
        let s = smooth_closed(&r, sg);
        if st.snap
            && let Some(c) = circle_fit(&s)
            && c.resid < st.circle_tol
            && c.r > 2.5
        {
            // Keep the loop's own direction and starting point: holes must stay opposite to
            // their outlines or the winding rule would add them instead of cancelling.
            if let Some((a0, a1)) = sweep(&s, c.c) {
                let dir = if a1 >= a0 { 1.0 } else { -1.0 };
                stats.circles += 1;
                return Some(arc_cubics(c.c, c.r, a0, a0 + dir * std::f64::consts::TAU));
            }
        }
        let (Some(&a), Some(&b), Some(&z)) = (s.first(), s.get(1), s.last()) else { return None };
        let t = b - z;
        let len = t.hypot();
        if len.is_nan() || len <= 1e-12 {
            return None;
        }
        let t = t / len;
        let mut q = s;
        q.push(a);
        return Some(fit_cubics(&q, t, -t, st.tol));
    }
    let mut segs: Vec<Cubic> = Vec::new();
    for (ci, &i0) in corners.iter().enumerate() {
        let i1 = *corners.get((ci + 1) % corners.len())?;
        let count = if i1 <= i0 { i1 + n - i0 } else { i1 - i0 } + 1;
        let run: Vec<Point> = (0..count).filter_map(|k| r.get((i0 + k) % n).copied()).collect();
        fit_run(&run, sg, st, stats, &mut segs);
    }
    (!segs.is_empty()).then_some(segs)
}

fn sane(segs: &[Cubic]) -> bool {
    !segs.is_empty() && segs.iter().all(|c| c.iter().all(|&p| finite(p)))
}

/// The outline `p` (a closed loop) as cubic Béziers, or `None` if it is too small or degenerate.
pub(crate) fn fit_outline(p: &[Point], st: &Style, stats: &mut Stats) -> Option<Vec<Cubic>> {
    if p.len() < 3 || !p.iter().all(|&q| finite(q)) {
        return None;
    }
    let mut local = Stats::default();
    // The reference is the raw outline, densified so the check never depends on how sparse it is
    // (and smoothed, for the contour of a smooth area).
    let reference = resample_closed(p, st.check_step / 0.3)
        .map_or_else(|| p.to_vec(), |(d, step)| if st.contour { smooth_closed(&d, st.sigma / step) } else { d });
    let checked = |st: &Style, local: &mut Stats| {
        styled_fit(p, st, local).filter(|s| sane(s)).filter(|s| {
            let samples = sample_cubics(s, st.check_step);
            all_within(&samples, &reference, st.max_dev) && all_within(&reference, &samples, st.max_dev)
        })
    };
    // A fit that strays is tried again without the clever parts before the plain fit, which
    // nothing checks.
    let fitted = checked(st, &mut local).or_else(|| {
        local = Stats::default();
        checked(&Style { snap: false, restore: None, ..*st }, &mut local)
    });
    match fitted {
        Some(segs) => {
            stats.restored += local.restored;
            stats.lines += local.lines;
            stats.arcs += local.arcs;
            stats.circles += local.circles;
            Some(segs)
        }
        None => {
            stats.reverted += 1;
            Some(plain_fit(p, st.tol)).filter(|s| sane(s))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logo::testutil::{circle, rounded_square};

    fn near_any(segs: &[Cubic], target: Point) -> f64 {
        sample_cubics(segs, 0.01).iter().map(|q| q.distance(target)).fold(f64::MAX, f64::min)
    }

    #[test]
    fn a_rounded_square_becomes_a_sharp_square_with_straight_sides() {
        let mut st = Stats::default();
        let segs = fit_outline(&rounded_square(0.7), &Style::default(), &mut st).expect("fit");
        assert_eq!(st.restored, 4);
        assert_eq!(st.lines, 4, "{st:?}");
        assert_eq!(segs.len(), 4, "a square is four lines");
        for (x, y) in [(10.0, 10.0), (30.0, 10.0), (30.0, 30.0), (10.0, 30.0)] {
            assert!(near_any(&segs, Point::new(x, y)) < 0.1);
        }
        assert_eq!(st.reverted, 0);
    }

    #[test]
    fn a_disc_becomes_a_true_circle() {
        let mut st = Stats::default();
        let segs = fit_outline(&circle(25.0, 25.0, 12.0, 700), &Style::default(), &mut st).expect("fit");
        assert_eq!(st.circles, 1);
        assert_eq!(segs.len(), 4);
        // Smoothing shrinks a curve by about sigma^2 / (2 r): 0.01 px here.
        for q in sample_cubics(&segs, 0.05) {
            assert!((q.distance(Point::new(25.0, 25.0)) - 12.0).abs() < 0.03);
        }
    }

    #[test]
    fn a_snapped_circle_keeps_the_direction_of_its_loop() {
        let forward = circle(25.0, 25.0, 12.0, 700);
        let mut backward = forward.clone();
        backward.reverse();
        let signed = |segs: &[Cubic]| {
            let p = sample_cubics(segs, 0.1);
            let n = p.len();
            (0..n).map(|i| p[i].x * p[(i + 1) % n].y - p[(i + 1) % n].x * p[i].y).sum::<f64>()
        };
        let (mut a, mut b) = (Stats::default(), Stats::default());
        let f = fit_outline(&forward, &Style::default(), &mut a).expect("forward");
        let r = fit_outline(&backward, &Style::default(), &mut b).expect("backward");
        assert_eq!((a.circles, b.circles), (1, 1));
        assert!(signed(&f) > 0.0 && signed(&r) < 0.0, "directions {} and {}", signed(&f), signed(&r));
    }

    #[test]
    fn without_snapping_a_circle_is_still_smooth_and_close() {
        let mut st = Stats::default();
        let style = Style { snap: false, restore: None, ..Style::default() };
        let segs = fit_outline(&circle(25.0, 25.0, 12.0, 700), &style, &mut st).expect("fit");
        assert_eq!(st.circles, 0);
        for q in sample_cubics(&segs, 0.05) {
            assert!((q.distance(Point::new(25.0, 25.0)) - 12.0).abs() < 0.2);
        }
    }

    #[test]
    fn the_guard_puts_back_a_plain_fit_when_the_clever_one_strays() {
        let mut st = Stats::default();
        // An absurdly tight limit makes every smoothed fit "stray".
        let style = Style { max_dev: 1e-6, ..Style::default() };
        let raw = rounded_square(0.7);
        let segs = fit_outline(&raw, &style, &mut st);
        assert_eq!(st.reverted, 1);
        assert!(segs.is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn a_short_run_between_corners_keeps_to_the_tolerance() {
        let run = [(0.0, 0.0), (0.12, 0.0), (0.24, 0.6), (0.36, 0.0), (0.48, 0.0)].map(|(x, y)| Point::new(x, y));
        let (st, sg) = (Style::default(), 0.5);
        let mut segs = Vec::new();
        fit_run(&run, sg, &st, &mut Stats::default(), &mut segs);
        let samples = sample_cubics(&segs, 0.001);
        for q in smooth_open(&run, sg) {
            let d = samples.iter().map(|s| s.distance(q)).fold(f64::MAX, f64::min);
            assert!(d <= st.tol + 1e-3, "{q:?} is {d} from the fit");
        }
    }

    #[test]
    fn a_plain_fit_keeps_a_ragged_outline_within_the_tolerance() {
        let raw: Vec<Point> = (0..120)
            .map(|i| {
                let a = i as f64 / 120.0 * std::f64::consts::TAU;
                let r = 12.0 + if i % 2 == 0 { 1.0 } else { -1.0 } * (0.2 + i as f64 * 0.02);
                Point::new(25.0 + r * a.cos(), 25.0 + r * a.sin())
            })
            .collect();
        let tol = Style::default().tol;
        let samples = sample_cubics(&plain_fit(&raw, tol), 0.005);
        for q in &raw {
            let d = samples.iter().map(|s| s.distance(*q)).fold(f64::MAX, f64::min);
            assert!(d <= tol + 1e-2, "{q:?} is {d} from the fit");
        }
    }

    #[test]
    fn degenerate_outlines_give_none_not_a_panic() {
        let mut st = Stats::default();
        let s = Style::default();
        assert!(fit_outline(&[], &s, &mut st).is_none());
        assert!(fit_outline(&[Point::new(1.0, 1.0); 3], &s, &mut st).is_none() || st.reverted > 0);
        assert!(fit_outline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0), Point::new(2.0, 0.0)], &s, &mut st).is_none());
        let tiny = [Point::new(0.0, 0.0), Point::new(0.01, 0.0), Point::new(0.0, 0.01)];
        let _ = fit_outline(&tiny, &s, &mut st);
        let _ = fit_outline(&circle(5.0, 5.0, 1e-9, 20), &s, &mut st);
        let _ = fit_outline(&circle(5.0, 5.0, 1e7, 50), &s, &mut st);
    }
}
