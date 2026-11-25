//! Where paths cross: [`crossings`] finds the places, [`insert_at`] puts anchors there (which
//! Split / Add Points at Crossings then keeps or cuts at), and [`crossings_on`] finds the places on
//! one path (which Trim at Crossings removes stretches between).

use std::collections::{BTreeMap, BTreeSet};

use craft_geom::{Anchor, AnchorKind, PathData, SubPath};
use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest, Point, Rect};

/// Most segments [`crossings`] compares at once.
pub const MAX_CROSSING_SEGMENTS: usize = 10_000;
/// Most subdivision steps one [`crossings`] call takes.
const MAX_STEPS: usize = 4_000_000;
/// A piece of curve whose inner control points lie this close (pt) to its chord counts as straight.
const FLAT: f64 = 1e-4;
/// Deepest a curve is halved.
const MAX_DEPTH: u32 = 96;
/// Crossings closer than this (pt) are one.
const SAME: f64 = 1e-3;
/// A parameter this close to a segment's end is its anchor.
const END: f64 = 1e-6;

/// A place on a path: (subpath, segment, t).
pub type Place = (usize, usize, f64);

pub(super) fn control_box(c: &CubicBez) -> Rect {
    Rect::from_points(c.p0, c.p1).union_pt(c.p2).union_pt(c.p3)
}

fn flat(c: &CubicBez) -> bool {
    let chord = c.p3 - c.p0;
    let len = chord.hypot();
    let off = |p: Point| if len > 1e-12 { chord.cross(p - c.p0).abs() / len } else { p.distance(c.p0) };
    off(c.p1) <= FLAT && off(c.p2) <= FLAT
}

/// Which ends of a segment are open ends of its path: (start, end).
type Open = (bool, bool);

/// The open ends of segment `s` of `sp`.
fn open_ends(sp: &SubPath, s: usize) -> Open {
    (!sp.closed && s == 0, !sp.closed && s + 1 == sp.segment_count())
}

/// Where the straight pieces `a` and `b` cross: (t on `a`, t on `b`), or `None` when their chords
/// don't cross (parallel ones never do, so paths lying along each other don't cross). An end in
/// `open_a` or `open_b` (start, end) that lies within [`SAME`] of the other piece meets it at the
/// point of that piece nearest it, at any angle.
fn pieces_cross(a: &CubicBez, b: &CubicBez, open_a: Open, open_b: Open) -> Option<(f64, f64)> {
    let (da, db) = (a.p3 - a.p0, b.p3 - b.p0);
    let den = da.cross(db);
    if den.abs() <= 1e-12 * da.hypot() * db.hypot() || den == 0.0 {
        return None;
    }
    let w = b.p0 - a.p0;
    let (s, u) = (w.cross(db) / den, w.cross(da) / den);
    let within = |v: f64| (-1e-9..=1.0 + 1e-9).contains(&v);
    if within(s) && within(u) {
        // The pieces are straight but their parameters needn't run evenly along them.
        let (s, u) = (s.clamp(0.0, 1.0), u.clamp(0.0, 1.0));
        return Some((a.nearest(a.p0.lerp(a.p3, s), 1e-9).t, b.nearest(b.p0.lerp(b.p3, u), 1e-9).t));
    }
    let ends_a = [(open_a.0, 0.0, a.p0), (open_a.1, 1.0, a.p3)].into_iter().filter(|e| e.0).map(|(_, ta, p)| {
        let n = b.nearest(p, 1e-9);
        ((ta, n.t), n.distance_sq)
    });
    let ends_b = [(open_b.0, 0.0, b.p0), (open_b.1, 1.0, b.p3)].into_iter().filter(|e| e.0).map(|(_, tb, p)| {
        let n = a.nearest(p, 1e-9);
        ((n.t, tb), n.distance_sq)
    });
    ends_a.chain(ends_b).filter(|e| e.1 < SAME * SAME).min_by(|x, y| x.1.total_cmp(&y.1)).map(|e| e.0)
}

/// `(s, t)` moved to where `a` and `b` cross exactly, by Newton's method on a(s) = b(t), or as it
/// was when that doesn't settle within both segments, closer than it started, and within the
/// distance a piece bent by [`FLAT`] can shift a crossing at their angle there.
fn refined(a: &CubicBez, b: &CubicBez, (s0, t0): (f64, f64)) -> (f64, f64) {
    let gap = |s: f64, t: f64| a.eval(s).distance(b.eval(t));
    let start = gap(s0, t0);
    if start <= 1e-12 || !start.is_finite() {
        return (s0, t0);
    }
    let (da, db) = (a.deriv(), b.deriv());
    let (mut s, mut t) = (s0, t0);
    for _ in 0..8 {
        let f = a.eval(s) - b.eval(t);
        let (u, v) = (da.eval(s).to_vec2(), db.eval(t).to_vec2());
        // a(s + ds) - b(t + dt) ≈ f + u ds - v dt, which is zero for these steps.
        let den = u.cross(v);
        let (ds, dt) = (-f.cross(v) / den, u.cross(f) / den);
        if !(ds.is_finite() && dt.is_finite()) {
            return (s0, t0);
        }
        (s, t) = (s + ds, t + dt);
        if !(-1e-12..=1.0 + 1e-12).contains(&s) || !(-1e-12..=1.0 + 1e-12).contains(&t) {
            return (s0, t0);
        }
        if gap(s, t) <= 1e-12 {
            break;
        }
    }
    let (s, t) = (s.clamp(0.0, 1.0), t.clamp(0.0, 1.0));
    let (u, v) = (da.eval(s).to_vec2(), db.eval(t).to_vec2());
    let sin = u.cross(v).abs() / (u.hypot() * v.hypot());
    let reach = 4.0 * FLAT / sin + SAME;
    let moved = a.eval(s).distance(a.eval(s0)).max(b.eval(t).distance(b.eval(t0)));
    if gap(s, t) < start.min(1e-9) && moved <= reach { (s, t) } else { (s0, t0) }
}

/// Where segments `a` and `b` cross: (t on `a`, t on `b`) pairs, halving both while their control
/// boxes meet until each piece is straight. `None` once `budget` steps are spent. `open_a` and
/// `open_b` are the segments' ends that are open ends of their paths (start, end): such an end
/// within [`SAME`] of the other segment meets it there, as a path cut where another crosses it
/// ends a little off that path.
fn segment_crossings(a: &CubicBez, b: &CubicBez, open_a: Open, open_b: Open, budget: &mut usize) -> Option<Vec<(f64, f64)>> {
    let mut stack = vec![(0.0, 1.0, 0.0, 1.0, 0u32)];
    let mut found: Vec<(f64, f64)> = vec![];
    while let Some((a0, a1, b0, b1, depth)) = stack.pop() {
        *budget = budget.checked_sub(1)?;
        let (ca, cb) = (a.subsegment(a0..a1), b.subsegment(b0..b1));
        if !control_box(&ca).inflate(SAME, SAME).overlaps(control_box(&cb)) {
            continue;
        }
        if (flat(&ca) && flat(&cb)) || depth >= MAX_DEPTH {
            // Only a piece's ends that are the segment's open ends reach past it.
            let (oa, ob) = ((open_a.0 && a0 == 0.0, open_a.1 && a1 == 1.0), (open_b.0 && b0 == 0.0, open_b.1 && b1 == 1.0));
            if let Some((ta, tb)) = pieces_cross(&ca, &cb, oa, ob) {
                found.push((a0 + ta * (a1 - a0), b0 + tb * (b1 - b0)));
            }
            continue;
        }
        // Halve the larger of the pieces still curved: two curves lying along each other then
        // split along their length, not into every pair of pieces.
        let size = |c: &CubicBez| if flat(c) { -1.0 } else { control_box(c).size().to_vec2().hypot() };
        let d = depth + 1;
        if size(&ca) >= size(&cb) {
            let am = (a0 + a1) / 2.0;
            stack.extend([(a0, am, b0, b1, d), (am, a1, b0, b1, d)]);
        } else {
            let bm = (b0 + b1) / 2.0;
            stack.extend([(a0, a1, b0, bm, d), (a0, a1, bm, b1, d)]);
        }
    }
    // A piece counts as straight while it bends by up to FLAT, so where the curves meet at a shallow
    // angle the chords cross some way along from where the curves do.
    for f in &mut found {
        *f = refined(a, b, *f);
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    // The same crossing found by neighbouring pieces counts once.
    found.dedup_by(|x, y| a.eval(x.0).distance(a.eval(y.0)) < SAME && b.eval(x.1).distance(b.eval(y.1)) < SAME);
    Some(found)
}

/// Where segment `c` crosses itself, as its two parameters in order: a cubic loops over itself at
/// most once. `None` when it doesn't, or only at a cusp or an end.
fn self_crossing(c: &CubicBez) -> Option<(f64, f64)> {
    // In powers of t, c(t) = a t³ + b t² + k t + p0. Two parameters s and t meet where
    // a (s² + st + t²) + b (s + t) + k = 0: with u = s + t and v = st, crossing that with `a` gives u,
    // and dotting it with `a` gives v; s and t are the roots of z² - u z + v.
    let (p0, p1, p2, p3) = (c.p0.to_vec2(), c.p1.to_vec2(), c.p2.to_vec2(), c.p3.to_vec2());
    let a = p3 - p2 * 3.0 + p1 * 3.0 - p0;
    let b = (p2 - p1 * 2.0 + p0) * 3.0;
    let k = (p1 - p0) * 3.0;
    let (ab, aa) = (a.cross(b), a.dot(a));
    // A curve with no loop can leave these near zero, however long and thin a loop is: the checks
    // at the end drop what that gives.
    if ab == 0.0 || aa == 0.0 {
        return None;
    }
    let u = -a.cross(k) / ab;
    let v = u * u + (b * u + k).dot(a) / aa;
    let disc = u * u - 4.0 * v;
    if !disc.is_finite() || disc <= 1e-12 {
        return None;
    }
    let (s, t) = ((u - disc.sqrt()) / 2.0, (u + disc.sqrt()) / 2.0);
    let inside = |x: f64| END < x && x < 1.0 - END;
    // The two places are one point, and the curve passes it in two directions (a straight curve
    // that doubles back on itself meets itself along its length, not at a crossing).
    let (ds, dt) = (c.deriv().eval(s).to_vec2(), c.deriv().eval(t).to_vec2());
    let across = ds.cross(dt).abs() > 1e-15 * ds.hypot() * dt.hypot();
    (inside(s) && inside(t) && across && c.eval(s).distance(c.eval(t)) < SAME).then_some((s, t))
}

/// For each segment of `sp`, and one past the last, the first segment at or after it that has a
/// length (the segment count when none has): segments with none lie at one point, between anchors
/// that coincide.
fn with_length_from(sp: &SubPath) -> Vec<usize> {
    let n = sp.segment_count();
    let mut out = vec![n; n + 1];
    for i in (0..n).rev() {
        let c = sp.segment(i);
        let next = if control_box(&c).size().to_vec2().hypot() < SAME { out.get(i + 1).copied().unwrap_or(n) } else { i };
        if let Some(o) = out.get_mut(i) {
            *o = next;
        }
    }
    out
}

/// The anchors segments `i` and `j` of `sp` share (`with_length` from [`with_length_from`]): none,
/// one, or both ends for the two segments of a closed subpath of two. Segments with no length between
/// them leave them meeting at that point.
fn shared_anchors(sp: &SubPath, with_length: &[usize], i: usize, j: usize) -> Vec<Point> {
    let (n, lo, hi) = (sp.segment_count(), i.min(j), i.max(j));
    let from = |k: usize| with_length.get(k).copied().unwrap_or(n);
    let mut out = vec![];
    if hi > lo && from(lo + 1) >= hi {
        out.extend([sp.segment(lo).p3, sp.segment(hi).p0]);
    }
    // Round the first anchor of a closed subpath.
    if sp.closed && n > 1 && hi > lo && from(hi + 1) >= n && from(0) >= lo {
        out.extend([sp.segment(hi).p3, sp.segment(lo).p0]);
    }
    out
}

/// Where segments `a` and `b` cross, without the places at the anchors in `shared`: two segments that
/// meet at an anchor don't cross there, but they can cross again elsewhere.
fn crossings_apart(a: (&CubicBez, Open), b: (&CubicBez, Open), shared: &[Point], budget: &mut usize) -> Option<Vec<(f64, f64)>> {
    let ((a, open_a), (b, open_b)) = (a, b);
    let mut found = segment_crossings(a, b, open_a, open_b, budget)?;
    found.retain(|(ta, _)| !shared.iter().any(|p| a.eval(*ta).distance(*p) < SAME));
    Some(found)
}

/// Where `paths` cross one another (and, with `self_crossings`, where each crosses itself, a segment
/// that loops over itself included): for each path, the places on it, in order. Segments that meet
/// at an anchor they share don't count as crossing there, and an open end of a path that lies
/// within 0.001 pt of another path (or of itself) meets it there; paths that lie along each other
/// don't cross. `None` past [`MAX_CROSSING_SEGMENTS`] segments or when the curves are too tangled
/// to sort out, or when a coordinate isn't finite.
pub fn crossings(paths: &[&PathData], self_crossings: bool) -> Option<Vec<Vec<Place>>> {
    let mut segs: Vec<(usize, usize, usize, CubicBez, Rect, Open)> = vec![];
    for (k, path) in paths.iter().enumerate() {
        for (si, sp) in path.subpaths.iter().enumerate() {
            for s in 0..sp.segment_count() {
                let c = sp.segment(s);
                if ![c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
                    return None;
                }
                segs.push((k, si, s, c, control_box(&c), open_ends(sp, s)));
                if segs.len() > MAX_CROSSING_SEGMENTS {
                    return None;
                }
            }
        }
    }
    let with_length: Vec<Vec<Vec<usize>>> = paths.iter().map(|p| p.subpaths.iter().map(with_length_from).collect()).collect();
    let mut out: Vec<Vec<Place>> = vec![vec![]; paths.len()];
    let mut budget = MAX_STEPS;
    for (i, (ka, sa, ia, ca, ra, oa)) in segs.iter().enumerate() {
        if self_crossings && let Some((s, t)) = self_crossing(ca) {
            out.get_mut(*ka)?.extend([(*sa, *ia, s), (*sa, *ia, t)]);
        }
        for (kb, sb, ib, cb, rb, ob) in segs.iter().skip(i + 1) {
            if (ka == kb && !self_crossings) || !ra.inflate(SAME, SAME).overlaps(*rb) {
                continue;
            }
            let shared = match (paths.get(*ka).and_then(|p| p.subpaths.get(*sa)), with_length.get(*ka).and_then(|w| w.get(*sa))) {
                (Some(sp), Some(w)) if ka == kb && sa == sb => shared_anchors(sp, w, *ia, *ib),
                _ => vec![],
            };
            for (ta, tb) in crossings_apart((ca, *oa), (cb, *ob), &shared, &mut budget)? {
                out.get_mut(*ka)?.push((*sa, *ia, ta));
                out.get_mut(*kb)?.push((*sb, *ib, tb));
            }
        }
    }
    for places in &mut out {
        places.sort_by(|x, y| (x.0, x.1).cmp(&(y.0, y.1)).then(x.2.total_cmp(&y.2)));
        places.dedup_by(|x, y| (x.0, x.1) == (y.0, y.1) && (x.2 - y.2).abs() < END);
    }
    Some(out)
}

/// Most segments of other paths [`crossings_on`] compares one path with.
pub const MAX_CROSSED_BY_SEGMENTS: usize = 200_000;
/// Most pairs of segments [`crossings_on`] looks at.
const MAX_PAIRS: usize = 60_000_000;

/// The work [`crossings_on_within`] may still do: pairs of segments to look at and subdivision steps.
/// One value shared by several calls bounds them all together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    pairs: usize,
    steps: usize,
}

impl Default for Work {
    /// The work one [`crossings_on`] call may do.
    fn default() -> Self {
        Self { pairs: MAX_PAIRS, steps: MAX_STEPS }
    }
}

impl Work {
    /// One `n`th of the work of [`Work::default`] (all of it for `n` of 0 or 1).
    pub const fn share(n: usize) -> Work {
        let n = if n == 0 { 1 } else { n };
        Work { pairs: MAX_PAIRS / n, steps: MAX_STEPS / n }
    }

    /// The work done since the budget was `before`.
    pub fn spent_since(self, before: Work) -> Work {
        Work { pairs: before.pairs.saturating_sub(self.pairs), steps: before.steps.saturating_sub(self.steps) }
    }

    /// Take `cost` (from [`Work::spent_since`]) out of what is left, as if that work were done
    /// again. `None`, leaving the budget as it was, when less than `cost` is left.
    pub fn draw(&mut self, cost: Work) -> Option<()> {
        let (pairs, steps) = (self.pairs.checked_sub(cost.pairs)?, self.steps.checked_sub(cost.steps)?);
        *self = Work { pairs, steps };
        Some(())
    }
}

/// Where `others` cross `path`, and where `path` crosses itself (a segment that loops over itself
/// included): the places on `path`, in order, within the work of [`Work::default`]. An open end of
/// a path that lies within 0.001 pt of another meets it there, as [`crossings_on_within`] says.
pub fn crossings_on(path: &PathData, others: &[&PathData]) -> Option<Vec<Place>> {
    crossings_on_within(path, others, &mut Work::default())
}

/// [`crossings_on`], drawing on `work`. Only the segments of `others` whose control boxes meet the
/// box of `path` are compared with it, so the paths far from it cost one box test per segment.
/// Segments of `path` that meet at an anchor they share don't count as crossing there, an open end
/// of a path that lies within 0.001 pt of another meets it there, and a segment of `others` with a
/// coordinate that isn't finite is left out. `None` when `path` has more than
/// [`MAX_CROSSING_SEGMENTS`] segments or a coordinate that isn't finite, when more than
/// [`MAX_CROSSED_BY_SEGMENTS`] segments of `others` lie in its box, when the pairs of segments to
/// look at are more than `work` has left, or when the curves are too tangled to sort out in the
/// steps it has left.
pub fn crossings_on_within(path: &PathData, others: &[&PathData], work: &mut Work) -> Option<Vec<Place>> {
    let mut own: Vec<(usize, usize, CubicBez, Rect, Open)> = vec![];
    for (si, sp) in path.subpaths.iter().enumerate() {
        for s in 0..sp.segment_count() {
            let c = sp.segment(s);
            if ![c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
                return None;
            }
            own.push((si, s, c, control_box(&c), open_ends(sp, s)));
            if own.len() > MAX_CROSSING_SEGMENTS {
                return None;
            }
        }
    }
    let Some(reach) = own.iter().map(|o| o.3).reduce(|a, b| a.union(b)).map(|r| r.inflate(SAME, SAME)) else { return Some(vec![]) };
    let mut near: Vec<(CubicBez, Rect, Open)> = vec![];
    for sp in others.iter().flat_map(|p| &p.subpaths) {
        for s in 0..sp.segment_count() {
            let c = sp.segment(s);
            let r = control_box(&c);
            if [c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.x.is_finite() && p.y.is_finite()) && reach.overlaps(r) {
                near.push((c, r, open_ends(sp, s)));
                if near.len() > MAX_CROSSED_BY_SEGMENTS {
                    return None;
                }
            }
        }
    }
    work.pairs = work.pairs.checked_sub(own.len().saturating_mul(near.len() + own.len() / 2))?;
    let with_length: Vec<Vec<usize>> = path.subpaths.iter().map(with_length_from).collect();
    let mut out: Vec<Place> = vec![];
    let budget = &mut work.steps;
    for (i, (sa, ia, ca, ra, oa)) in own.iter().enumerate() {
        let ra = ra.inflate(SAME, SAME);
        if let Some((s, t)) = self_crossing(ca) {
            out.extend([(*sa, *ia, s), (*sa, *ia, t)]);
        }
        for (cb, rb, ob) in &near {
            if ra.overlaps(*rb) {
                out.extend(segment_crossings(ca, cb, *oa, *ob, budget)?.into_iter().map(|(ta, _)| (*sa, *ia, ta)));
            }
        }
        for (sb, ib, cb, rb, ob) in own.iter().skip(i + 1) {
            if !ra.overlaps(*rb) {
                continue;
            }
            let shared = match (path.subpaths.get(*sa), with_length.get(*sa)) {
                (Some(sp), Some(w)) if sa == sb => shared_anchors(sp, w, *ia, *ib),
                _ => vec![],
            };
            for (ta, tb) in crossings_apart((ca, *oa), (cb, *ob), &shared, budget)? {
                out.push((*sa, *ia, ta));
                out.push((*sb, *ib, tb));
            }
        }
    }
    out.sort_by(|x, y| (x.0, x.1).cmp(&(y.0, y.1)).then(x.2.total_cmp(&y.2)));
    out.dedup_by(|x, y| (x.0, x.1) == (y.0, y.1) && (x.2 - y.2).abs() < END);
    Some(out)
}

/// `sp` with an anchor at each of `places` (segment, t), the shape unchanged → (the subpath, the
/// indexes of the anchors at those places: the new ones, and existing ones where a place falls on
/// an anchor). Places off the subpath are left out.
pub fn insert_at(sp: &SubPath, places: &[(usize, f64)]) -> (SubPath, BTreeSet<usize>) {
    let (n, segs) = (sp.anchors.len(), sp.segment_count());
    let mut inner: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    let mut at_anchor = vec![];
    for &(seg, t) in places.iter().filter(|(seg, t)| *seg < segs && t.is_finite()) {
        match t {
            _ if t <= END => at_anchor.push(seg),
            _ if t >= 1.0 - END => at_anchor.push((seg + 1) % n),
            _ => inner.entry(seg).or_default().push(t),
        }
    }
    for ts in inner.values_mut() {
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|x, y| (*x - *y).abs() < END);
    }
    // Each split segment's pieces, cut from the original curve so their parameters stay true.
    let mut anchors = sp.anchors.clone();
    let mut added: BTreeMap<usize, Vec<Anchor>> = BTreeMap::new();
    for (&seg, ts) in &inner {
        let c = sp.segment(seg);
        let bounds: Vec<f64> = std::iter::once(0.0).chain(ts.iter().copied()).chain([1.0]).collect();
        let pieces: Vec<CubicBez> = bounds.windows(2).filter_map(|w| Some(c.subsegment(*w.first()?..*w.get(1)?))).collect();
        if sp.segment_is_line(seg) {
            added.insert(seg, pieces.iter().skip(1).map(|q| Anchor::corner(q.p0)).collect());
            continue;
        }
        let made = pieces.windows(2).filter_map(|w| match w {
            [a, b] => Some(Anchor { p: b.p0, h_in: a.p2, h_out: b.p1, kind: AnchorKind::Smooth }),
            _ => None,
        });
        added.insert(seg, made.collect());
        if let (Some(first), Some(a)) = (pieces.first(), anchors.get_mut(seg)) {
            a.h_out = first.p1;
        }
        if let (Some(last), Some(a)) = (pieces.last(), anchors.get_mut((seg + 1) % n)) {
            a.h_in = last.p2;
        }
    }
    let mut out = SubPath::new(Vec::with_capacity(n + places.len()), sp.closed);
    for (i, a) in anchors.into_iter().enumerate() {
        out.anchors.push(a);
        out.anchors.extend(added.remove(&i).unwrap_or_default());
    }
    // How many anchors were added before each anchor, counted in one pass for all of them.
    let mut before = vec![0usize; n + 1];
    for (&seg, ts) in &inner {
        if let Some(b) = before.get_mut(seg + 1) {
            *b += ts.len();
        }
    }
    let mut sum = 0;
    for b in &mut before {
        sum += *b;
        *b = sum;
    }
    let added_before = |j: usize| before.get(j).copied().unwrap_or(0);
    let mut marks: BTreeSet<usize> = at_anchor.into_iter().map(|j| j + added_before(j)).collect();
    for (&seg, ts) in &inner {
        let base = seg + added_before(seg);
        marks.extend((1..=ts.len()).map(|m| base + m));
    }
    (out, marks)
}
