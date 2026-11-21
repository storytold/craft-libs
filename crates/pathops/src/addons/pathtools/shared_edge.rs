//! Merging two touching edges: where two paths meet at two pairs of coincident anchors, the
//! stretches of path between those anchors become one shared edge ([`stretches_at`]); where the
//! anchors aren't there yet, near ones are brought together and a missing one is added
//! ([`ends_at`]). [`merge_preview`] says what a plan would do and [`apply_merge`] does it. The
//! Shared Edge Merge Tool and `addon.pathtools.mergeEdge` drive both from the same plan.

use std::collections::BTreeMap;

use craft_geom::{Anchor, AnchorKind, PathData, SubPath};
use kurbo::{CubicBez, ParamCurve, ParamCurveArclen, ParamCurveNearest, Point};

use super::crossings::control_box;
use super::extend::finite;
use super::{ACCURACY, TINY, insert_at, unit};
use crate::fit::{end_tangent, is_straight, start_tangent};

#[cfg(test)]
mod tests;

/// How near two anchors count as one, by default and at most (pt).
pub const COINCIDENCE: f64 = 0.002;
pub const MAX_COINCIDENCE: f64 = 3.0;

/// Stretches differing in length by more than this ratio aren't the same edge.
const MAX_LENGTH_RATIO: f64 = 1.1;
/// The cosine of the widest angle (120°) stretches may make at an end: past it they run apart.
const MIN_END_COS: f64 = -0.5;
/// Paths with more anchors than this aren't searched.
const MAX_PATH_ANCHORS: usize = 200_000;
/// Most segments a stretch runs.
const MAX_STRETCH: usize = 5_000;
/// Most segments under the brush looked at on each path.
const MAX_NEAR: usize = 16;
/// Anchors of the two stretches closer than this share of the length are one anchor of the result.
const SAME_PLACE: f64 = 0.01;
/// How close (pt along a segment) a cut has to be to the segment's end to be that end.
const AT_END: f64 = 1e-6;
/// The cosine of the widest bend (5°) the average's joints are straightened out of.
const MIN_JOINT_COS: f64 = 0.9962;
/// Control points this close (pt) are the same edge.
const SAME_EDGE: f64 = 1e-6;

/// Which outline a merge keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// Half way between the two.
    Average,
    /// The first path's: it doesn't change.
    First,
    /// The second path's.
    Second,
}

impl Follow {
    pub const ALL: [Self; 3] = [Self::Average, Self::First, Self::Second];

    pub fn id(self) -> &'static str {
        match self {
            Self::Average => "average",
            Self::First => "a",
            Self::Second => "b",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == s)
    }
}

/// `segments` segments of a subpath from anchor `start` forward; `reversed` when the merge runs
/// along them backward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub subpath: usize,
    pub start: usize,
    pub segments: usize,
    pub reversed: bool,
}

/// One path's side of making an end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EndSide {
    /// The anchor it has there.
    Anchor { subpath: usize, anchor: usize },
    /// It has none: one goes on this segment.
    Insert { subpath: usize, segment: usize, t: f64 },
}

/// What a click can do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MergePlan {
    /// Replace these two stretches, running the same way, with one edge.
    Edge { a: Stretch, b: Stretch },
    /// Bring the two sides to one point.
    Ends { a: EndSide, b: EndSide },
}

/// What a plan would do.
#[derive(Clone, Debug, PartialEq)]
pub enum MergePreview {
    /// The two stretches and the edge replacing them; `done` when they are that edge already.
    Edge { a: Vec<CubicBez>, b: Vec<CubicBez>, merged: Vec<CubicBez>, done: bool },
    /// Where the two sides are and where they meet.
    Ends { a: Point, b: Point, to: Point },
}

fn searchable(a: &PathData, b: &PathData, at: Point) -> bool {
    let fits = |p: &PathData| p.anchor_count() <= MAX_PATH_ANCHORS && p.subpaths.iter().all(finite);
    at.x.is_finite() && at.y.is_finite() && fits(a) && fits(b)
}

impl Stretch {
    /// Its segments in the order the merge runs along them.
    fn chain(&self, path: &PathData) -> Option<Vec<CubicBez>> {
        let sp = path.subpaths.get(self.subpath)?;
        let count = sp.segment_count();
        if count == 0 || self.segments == 0 || self.segments > count || self.start >= sp.anchors.len() {
            return None;
        }
        // Only a closed subpath runs on past its last segment: a plan made for a longer open
        // subpath (the path was edited since) would wrap and splice the wrong anchors.
        if !sp.closed && self.start + self.segments > count {
            return None;
        }
        let forward: Vec<CubicBez> = (0..self.segments).map(|k| sp.segment((self.start + k) % count)).collect();
        Some(if self.reversed { backward(&forward) } else { forward })
    }
}

/// `chain` run the other way.
fn backward(chain: &[CubicBez]) -> Vec<CubicBez> {
    chain.iter().rev().map(|c| CubicBez::new(c.p3, c.p2, c.p1, c.p0)).collect()
}

fn length(chain: &[CubicBez]) -> f64 {
    chain.iter().map(|c| c.arclen(ACCURACY)).sum()
}

/// A subpath's anchors in order of x, to find the ones at a point quickly.
struct ByX<'a> {
    sp: &'a SubPath,
    order: Vec<(f64, usize)>,
}

impl<'a> ByX<'a> {
    fn new(sp: &'a SubPath) -> Self {
        let mut order: Vec<(f64, usize)> = sp.anchors.iter().enumerate().map(|(i, a)| (a.p.x, i)).collect();
        order.sort_by(|x, y| x.0.total_cmp(&y.0));
        Self { sp, order }
    }

    /// Its anchor nearest `p`, within `tol`.
    fn partner(&self, p: Point, tol: f64) -> Option<usize> {
        let from = self.order.partition_point(|(x, _)| *x < p.x - tol);
        let within = self.order.iter().skip(from).take_while(|(x, _)| *x <= p.x + tol);
        let near = within.filter_map(|(_, i)| Some((self.sp.anchors.get(*i)?.p.distance(p), *i))).filter(|(d, _)| *d <= tol);
        near.min_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, i)| i)
    }
}

/// The segments of `path` within `radius` of `at`, nearest first: (distance, subpath, segment).
fn segments_near(path: &PathData, at: Point, radius: f64) -> Vec<(f64, usize, usize)> {
    let mut out = vec![];
    for (si, sp) in path.subpaths.iter().enumerate() {
        for s in 0..sp.segment_count() {
            let c = sp.segment(s);
            if !control_box(&c).inflate(radius, radius).contains(at) {
                continue;
            }
            let d = c.nearest(at, ACCURACY).distance_sq.sqrt();
            if d <= radius {
                out.push((d, si, s));
            }
        }
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out.truncate(MAX_NEAR);
    out
}

/// The stretch of `sa` around segment `seg` between the nearest anchors each way that `b` has a
/// coincident one for, and the stretch of `b` between those that holds its segment `seg_b`.
fn matching((si_a, sa): (usize, &SubPath), seg: usize, (si_b, by_x): (usize, &ByX), seg_b: usize, tol: f64) -> Option<(Stretch, Stretch)> {
    let (n, m) = (sa.anchors.len(), by_x.sp.anchors.len());
    let limit = sa.segment_count().min(MAX_STRETCH);
    let partner = |i: usize| by_x.partner(sa.anchors.get(i)?.p, tol);
    let back = (0..limit).take_while(|k| sa.closed || *k <= seg).find_map(|k| Some((k, partner((seg + n - k) % n)?)));
    let ahead = (0..limit).take_while(|k| sa.closed || seg + 1 + k < n).find_map(|k| Some((k, partner((seg + 1 + k) % n)?)));
    let ((back, b0), (ahead, b1)) = (back?, ahead?);
    let segments = back + ahead + 1;
    // Round to the same anchor, or to the same one of the other path: there is no stretch between.
    if b0 == b1 || (sa.closed && segments >= n) {
        return None;
    }
    let first = Stretch { subpath: si_a, start: (seg + n - back) % n, segments, reversed: false };
    let run = |from: usize, to: usize| (to + m - from) % m;
    let holds = |from: usize, to: usize| (seg_b + m - from) % m < run(from, to);
    let second = if (by_x.sp.closed || b0 < b1) && holds(b0, b1) {
        Stretch { subpath: si_b, start: b0, segments: run(b0, b1), reversed: false }
    } else if (by_x.sp.closed || b1 < b0) && holds(b1, b0) {
        Stretch { subpath: si_b, start: b1, segments: run(b1, b0), reversed: true }
    } else {
        return None;
    };
    Some((first, second))
}

/// Could these two run along one another: much the same length, not leaving their ends in
/// opposite directions?
fn alike(a: &[CubicBez], b: &[CubicBez]) -> bool {
    let (la, lb) = (length(a), length(b));
    let ((Some(a0), Some(a1)), (Some(b0), Some(b1))) = ((a.first(), a.last()), (b.first(), b.last())) else { return false };
    la.min(lb) > TINY
        && la.max(lb) <= la.min(lb) * MAX_LENGTH_RATIO
        && start_tangent(a0).dot(start_tangent(b0)) >= MIN_END_COS
        && end_tangent(a1).dot(end_tangent(b1)) >= MIN_END_COS
}

/// The two stretches of `a` and `b` that can merge within `radius` of `at`, each pair of ends
/// coincident within `tol`: the pair nearest `at`. `None` when there is none.
pub fn stretches_at(a: &PathData, b: &PathData, at: Point, radius: f64, tol: f64) -> Option<MergePlan> {
    if !searchable(a, b, at) {
        return None;
    }
    let (near_a, near_b) = (segments_near(a, at, radius), segments_near(b, at, radius));
    let index: BTreeMap<usize, ByX> = near_b.iter().filter_map(|&(_, si, _)| Some((si, ByX::new(b.subpaths.get(si)?)))).collect();
    let mut pairs: Vec<_> = near_a.iter().flat_map(|&(da, sa, ga)| near_b.iter().map(move |&(db, sb, gb)| (da + db, (sa, ga), (sb, gb)))).collect();
    pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
    pairs.into_iter().find_map(|(_, (sa, ga), (sb, gb))| {
        let (x, y) = matching((sa, a.subpaths.get(sa)?), ga, (sb, index.get(&sb)?), gb, tol)?;
        alike(&x.chain(a)?, &y.chain(b)?).then_some(MergePlan::Edge { a: x, b: y })
    })
}

/// The anchor of `path` nearest `at`, within `radius`: (subpath, anchor, where).
fn anchor_near(path: &PathData, at: Point, radius: f64) -> Option<(usize, usize, Point)> {
    let near = path.anchors().map(|(si, ai, a)| (a.p.distance(at), si, ai, a.p)).filter(|x| x.0 <= radius);
    near.min_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, si, ai, p)| (si, ai, p))
}

/// Where `path` takes an anchor to match one at `p`, if the place is within `radius` of `at`.
fn matching_place(path: &PathData, p: Point, at: Point, radius: f64) -> Option<EndSide> {
    let (subpath, segment, t, q, _) = path.nearest(p)?;
    (q.distance(at) <= radius).then_some(EndSide::Insert { subpath, segment, t })
}

/// What making an end within `radius` of `at` takes: the two paths' nearest anchors there, or the
/// anchor one has and the place on the other for its match. `None` when neither has an anchor
/// there, or they are together already.
pub fn ends_at(a: &PathData, b: &PathData, at: Point, radius: f64) -> Option<MergePlan> {
    if !searchable(a, b, at) {
        return None;
    }
    let anchor = |(subpath, anchor, _): (usize, usize, Point)| EndSide::Anchor { subpath, anchor };
    match (anchor_near(a, at, radius), anchor_near(b, at, radius)) {
        (Some(x), Some(y)) => (x.2.distance(y.2) > TINY).then(|| MergePlan::Ends { a: anchor(x), b: anchor(y) }),
        (Some(x), None) => Some(MergePlan::Ends { a: anchor(x), b: matching_place(b, x.2, at, radius)? }),
        (None, Some(y)) => Some(MergePlan::Ends { a: matching_place(a, y.2, at, radius)?, b: anchor(y) }),
        (None, None) => None,
    }
}

/// Where the anchors between `chain`'s segments fall, as shares of its length.
fn fractions(chain: &[CubicBez]) -> Vec<f64> {
    let total = length(chain);
    let mut run = 0.0;
    let inner = chain.iter().take(chain.len().saturating_sub(1));
    inner
        .map(|c| {
            run += c.arclen(ACCURACY);
            run / total
        })
        .collect()
}

/// The places (share of the length on each stretch) both stretches get an anchor: each one's own
/// anchors, where two nearly meet the pair of them, and otherwise the same share on the other.
fn shared_cuts(fa: &[f64], fb: &[f64]) -> Vec<(f64, f64)> {
    let (mut i, mut j) = (0, 0);
    let mut out = vec![];
    loop {
        match (fa.get(i), fb.get(j)) {
            (Some(&x), Some(&y)) if (x - y).abs() <= SAME_PLACE => {
                out.push((x, y));
                i += 1;
                j += 1;
            }
            (Some(&x), Some(&y)) if x < y => {
                out.push((x, x));
                i += 1;
            }
            (Some(&x), None) => {
                out.push((x, x));
                i += 1;
            }
            (_, Some(&y)) => {
                out.push((y, y));
                j += 1;
            }
            (None, None) => return out,
        }
    }
}

/// `chain` with a joint at each of the shares of its length `at`; those at a joint already add none.
fn cut(chain: &[CubicBez], mut at: Vec<f64>) -> Vec<CubicBez> {
    at.sort_by(f64::total_cmp);
    let total = length(chain);
    let mut next = at.into_iter().peekable();
    let mut run = 0.0;
    let mut out = vec![];
    for c in chain {
        let len = c.arclen(ACCURACY);
        let mut ts = vec![0.0];
        while let Some(along) = next.peek().map(|f| f * total - run).filter(|d| *d < len - AT_END) {
            next.next();
            if along > AT_END {
                ts.push(c.inv_arclen(along, ACCURACY));
            }
        }
        ts.push(1.0);
        out.extend(ts.windows(2).filter_map(|w| Some(c.subsegment(*w.first()?..*w.get(1)?))));
        run += len;
    }
    out
}

/// The curve half way between `a` and `b`; a straight one has its handles on its anchors.
fn halfway(a: &CubicBez, b: &CubicBez) -> CubicBez {
    let c = CubicBez::new(a.p0.midpoint(b.p0), a.p1.midpoint(b.p1), a.p2.midpoint(b.p2), a.p3.midpoint(b.p3));
    if is_straight(&c, SAME_EDGE) { CubicBez::new(c.p0, c.p0, c.p3, c.p3) } else { c }
}

/// Bring the handles at `chain`'s joints in line where they all but are: averaging leaves a small
/// bend at what was a smooth anchor on both stretches.
fn align_joints(chain: &mut [CubicBez]) {
    for k in 1..chain.len() {
        let (before, after) = chain.split_at_mut(k);
        let (Some(x), Some(y)) = (before.last_mut(), after.first_mut()) else { continue };
        let (Some(i), Some(o)) = (unit(x.p3 - x.p2), unit(y.p1 - y.p0)) else { continue };
        if i.dot(o) < MIN_JOINT_COS {
            continue;
        }
        let Some(dir) = unit(i + o) else { continue };
        x.p2 = x.p3 - dir * (x.p3 - x.p2).hypot();
        y.p1 = y.p0 + dir * (y.p1 - y.p0).hypot();
    }
}

/// The edge replacing stretches `a` and `b`, which run the same way. `None` when their anchors
/// can't be matched up.
fn merged_edge(a: &[CubicBez], b: &[CubicBez], follow: Follow) -> Option<Vec<CubicBez>> {
    match follow {
        Follow::First => return Some(a.to_vec()),
        Follow::Second => return Some(b.to_vec()),
        Follow::Average => {}
    }
    // A segment of no length has nothing to average.
    let solid = |c: &[CubicBez]| -> Vec<CubicBez> { c.iter().filter(|s| s.arclen(ACCURACY) > TINY).copied().collect() };
    let (a, b) = (solid(a), solid(b));
    let cuts = shared_cuts(&fractions(&a), &fractions(&b));
    let (pa, pb) = (cut(&a, cuts.iter().map(|c| c.0).collect()), cut(&b, cuts.iter().map(|c| c.1).collect()));
    if pa.is_empty() || pa.len() != pb.len() {
        return None;
    }
    let mut out: Vec<CubicBez> = pa.iter().zip(&pb).map(|(x, y)| halfway(x, y)).collect();
    align_joints(&mut out);
    Some(out)
}

fn in_line(a: &Anchor) -> bool {
    match (unit(a.h_out - a.p), unit(a.p - a.h_in)) {
        (Some(o), Some(i)) => o.cross(i).abs() <= 1e-6 && o.dot(i) > 0.0,
        _ => false,
    }
}

/// `sp` with `st` replaced by `chain`, which runs the way the merge does: the anchors inside are
/// `chain`'s joints and the two at the ends move to `chain`'s ends, their outer handles with them.
/// An end whose handles no longer line up turns into a corner.
fn splice(sp: &SubPath, st: Stretch, chain: &[CubicBez]) -> Option<SubPath> {
    let own = backward(chain);
    let chain = if st.reversed { own.as_slice() } else { chain };
    let (first, last) = (chain.first()?, chain.last()?);
    let mut out = sp.anchors.to_vec();
    let start = st.start.min(out.len());
    out.rotate_left(start);
    let settle = |a: &mut Anchor| {
        if a.kind == AnchorKind::Smooth && !in_line(a) {
            a.kind = AnchorKind::Corner;
        }
    };
    let a = out.first_mut()?;
    a.translate(first.p0 - a.p);
    a.h_out = first.p1;
    settle(a);
    let b = out.get_mut(st.segments)?;
    b.translate(last.p3 - b.p);
    b.h_in = last.p2;
    settle(b);
    let joints = chain.windows(2).filter_map(|w| match w {
        [x, y] => {
            let mut a = Anchor { p: y.p0, h_in: x.p2, h_out: y.p1, kind: AnchorKind::Corner };
            if in_line(&a) {
                a.kind = AnchorKind::Smooth;
            }
            Some(a)
        }
        _ => None,
    });
    out.splice(1..st.segments, joints);
    // Back in its place; a stretch over the start of a closed subpath only shifts where it starts.
    let start = start.min(out.len());
    out.rotate_right(start);
    Some(SubPath::new(out, sp.closed))
}

/// Where a side of an end is now.
fn side_at(path: &PathData, side: EndSide) -> Option<Point> {
    match side {
        EndSide::Anchor { subpath, anchor } => Some(path.subpaths.get(subpath)?.anchors.get(anchor)?.p),
        EndSide::Insert { subpath, segment, t } => {
            let sp = path.subpaths.get(subpath)?;
            (segment < sp.segment_count()).then(|| sp.segment(segment).eval(t.clamp(0.0, 1.0)))
        }
    }
}

/// Where the two sides of an end meet: on the path followed; with neither, half way between two
/// anchors, or at the anchor when the other side is a place on a path.
fn meeting_point((a, sa): (Point, EndSide), (b, sb): (Point, EndSide), follow: Follow) -> Point {
    match (follow, sa, sb) {
        (Follow::First, ..) | (Follow::Average, EndSide::Anchor { .. }, EndSide::Insert { .. }) => a,
        (Follow::Second, ..) | (Follow::Average, EndSide::Insert { .. }, EndSide::Anchor { .. }) => b,
        _ => a.midpoint(b),
    }
}

/// `path` with `side`'s anchor (added if it lacks one) at `to`.
fn put_side(path: &PathData, side: EndSide, to: Point) -> Option<PathData> {
    let mut out = path.clone();
    let anchor = match side {
        EndSide::Anchor { subpath, anchor } => out.anchor_mut(subpath, anchor)?,
        EndSide::Insert { subpath, segment, t } => {
            let sp = out.subpaths.get_mut(subpath)?;
            let (new, added) = insert_at(sp, &[(segment, t)]);
            *sp = new;
            sp.anchors.get_mut(*added.first()?)?
        }
    };
    anchor.translate(to - anchor.p);
    Some(out)
}

fn same_edge(x: &[CubicBez], y: &[CubicBez]) -> bool {
    let near = |p: Point, q: Point| p.distance(q) <= SAME_EDGE;
    x.len() == y.len() && x.iter().zip(y).all(|(c, d)| near(c.p0, d.p0) && near(c.p1, d.p1) && near(c.p2, d.p2) && near(c.p3, d.p3))
}

/// What `plan` would do to `a` and `b`, following `follow`.
pub fn merge_preview(a: &PathData, b: &PathData, plan: &MergePlan, follow: Follow) -> Option<MergePreview> {
    match *plan {
        MergePlan::Edge { a: sa, b: sb } => {
            let (ca, cb) = (sa.chain(a)?, sb.chain(b)?);
            let merged = merged_edge(&ca, &cb, follow)?;
            let done = same_edge(&merged, &ca) && same_edge(&merged, &cb);
            Some(MergePreview::Edge { a: ca, b: cb, merged, done })
        }
        MergePlan::Ends { a: ea, b: eb } => {
            let (pa, pb) = (side_at(a, ea)?, side_at(b, eb)?);
            Some(MergePreview::Ends { a: pa, b: pb, to: meeting_point((pa, ea), (pb, eb), follow) })
        }
    }
}

/// `a` and `b` as `plan` leaves them, following `follow`: each subpath stays closed or open, and
/// the stretch of the path followed doesn't change. `None` when the plan doesn't fit them.
pub fn apply_merge(a: &PathData, b: &PathData, plan: &MergePlan, follow: Follow) -> Option<(PathData, PathData)> {
    match (*plan, merge_preview(a, b, plan, follow)?) {
        (MergePlan::Edge { a: sa, b: sb }, MergePreview::Edge { merged, .. }) => {
            let replaced = |path: &PathData, st: Stretch, keep: Follow| {
                let mut out = path.clone();
                if follow != keep {
                    let sp = out.subpaths.get_mut(st.subpath)?;
                    *sp = splice(sp, st, &merged)?;
                }
                Some(out)
            };
            Some((replaced(a, sa, Follow::First)?, replaced(b, sb, Follow::Second)?))
        }
        (MergePlan::Ends { a: ea, b: eb }, MergePreview::Ends { to, .. }) => Some((put_side(a, ea, to)?, put_side(b, eb, to)?)),
        _ => None,
    }
}
