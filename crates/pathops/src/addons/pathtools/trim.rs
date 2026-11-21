//! The stretches of a subpath between the places it is crossed ([`spans`]), the stretch a place
//! lies on ([`span_at`]), a stretch as a subpath of its own ([`slice`]) and the subpath with some
//! of its stretches removed ([`without_spans`]).

use craft_geom::{Anchor, AnchorKind, SubPath};
use kurbo::{ParamCurve, Point};

/// A parameter this close to a segment's end is its anchor.
const END: f64 = 1e-6;
/// Places closer than this (pt) are one.
const SAME: f64 = 1e-3;

/// A stretch of a subpath: where it starts and ends, each a segment's index plus the parameter on
/// that segment. On a closed subpath the end is past the segment count where the stretch runs
/// through the first anchor.
pub type Span = (f64, f64);

/// The point at position `u` (a segment's index plus the parameter on it) of `sp`, which has
/// segments. A closed subpath's positions wrap round.
fn point_at(sp: &SubPath, u: f64) -> Point {
    let n = sp.segment_count();
    let u = if sp.closed { u.rem_euclid(n as f64) } else { u.clamp(0.0, n as f64) };
    let seg = (u.floor() as usize).min(n.saturating_sub(1));
    sp.segment(seg).eval(u - seg as f64)
}

/// The stretches `places` (segment, t) divide `sp` into, in the subpath's order. An open subpath
/// starts and ends a stretch at its ends; a closed one with no place is one stretch, the whole of
/// it, and so is a closed one with one place, from that place round to it again. Places the subpath
/// runs between without moving 0.001 pt away are one (a loop's two visits to a point are two), so
/// are a place and an open end, and places off the subpath are left out.
pub fn spans(sp: &SubPath, places: &[(usize, f64)]) -> Vec<Span> {
    let n = sp.segment_count();
    if n == 0 {
        return vec![];
    }
    let nf = n as f64;
    let mut at: Vec<f64> = places
        .iter()
        .filter(|(seg, t)| *seg < n && t.is_finite())
        .map(|&(seg, t)| {
            let t = if t <= END {
                0.0
            } else if t >= 1.0 - END {
                1.0
            } else {
                t
            };
            let u = seg as f64 + t;
            if sp.closed && u >= nf { u - nf } else { u }
        })
        .collect();
    at.sort_by(f64::total_cmp);
    // Two positions are one place when the subpath stays at the same point between them; a loop
    // that comes back to a point visits it twice.
    let same = |a: f64, b: f64| {
        let p = point_at(sp, a);
        (b - a).abs() < 1.0 && [0.25, 0.5, 0.75, 1.0].iter().all(|f| point_at(sp, a + (b - a) * f).distance(p) < SAME)
    };
    at.dedup_by(|later, earlier| same(*earlier, *later));
    if !sp.closed {
        at.retain(|u| !same(0.0, *u) && !same(*u, nf));
        let bounds: Vec<f64> = std::iter::once(0.0).chain(at).chain([nf]).collect();
        return bounds.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect();
    }
    // The last place and the first meet round the first anchor.
    while let (Some(&first), Some(&last)) = (at.first(), at.last())
        && at.len() > 1
        && same(last - nf, first)
    {
        at.pop();
    }
    let Some(&first) = at.first() else { return vec![(0.0, nf)] };
    at.push(first + nf);
    at.windows(2).filter_map(|w| Some((*w.first()?, *w.get(1)?))).collect()
}

/// Which of `stretches` of `sp` the place (`seg`, `t`) lies on. A place where two stretches meet
/// is on the one that starts there.
pub fn span_at(sp: &SubPath, stretches: &[Span], seg: usize, t: f64) -> Option<usize> {
    let nf = sp.segment_count() as f64;
    if !t.is_finite() {
        return None;
    }
    let u = (seg as f64 + t.clamp(0.0, 1.0)).min(nf);
    // On a closed subpath the closing segment's end is the first anchor.
    let u = if sp.closed && u >= nf { u - nf } else { u };
    let on = |&(from, to): &Span| (from <= u && u < to) || (sp.closed && from <= u + nf && u + nf < to);
    stretches.iter().position(on).or_else(|| (!sp.closed && u >= nf).then(|| stretches.len().checked_sub(1)).flatten())
}

/// The stretch `(from, to)` of `sp` as an open subpath of its own: the anchors it runs through keep
/// their kind and handles, and its ends are corners. Empty when the stretch has no length.
pub fn slice(sp: &SubPath, (from, to): Span) -> SubPath {
    let n = sp.segment_count();
    let mut anchors: Vec<Anchor> = vec![];
    if n == 0 || !(from.is_finite() && to.is_finite()) || from < 0.0 || to <= from {
        return SubPath::new(anchors, false);
    }
    let to = to.min(from + n as f64);
    let mut i = from.floor() as usize;
    while (i as f64) < to {
        let (t0, t1) = ((from - i as f64).max(0.0), (to - i as f64).min(1.0));
        let seg = i % n;
        i += 1;
        if t1 <= t0 {
            continue;
        }
        let whole = sp.segment(seg);
        let c = if t0 <= 0.0 && t1 >= 1.0 { whole } else { whole.subsegment(t0..t1) };
        let (h1, h2) = if sp.segment_is_line(seg) { (c.p0, c.p3) } else { (c.p1, c.p2) };
        match anchors.last_mut() {
            Some(a) => a.h_out = h1,
            None => anchors.push(Anchor { p: c.p0, h_in: c.p0, h_out: h1, kind: AnchorKind::Corner }),
        }
        // An anchor the stretch runs through keeps its kind.
        let through = t1 >= 1.0 && (i as f64) < to;
        let kind = if through { sp.anchors.get((seg + 1) % sp.anchors.len()).map_or(AnchorKind::Corner, |a| a.kind) } else { AnchorKind::Corner };
        anchors.push(Anchor { p: c.p3, h_in: h2, h_out: c.p3, kind });
    }
    SubPath::new(anchors, false)
}

/// `sp` without the stretches of `stretches` that `remove` marks: the pieces left, each an open
/// subpath, in the subpath's order. Stretches left side by side stay one piece, so no anchor is
/// added where a stretch that stays meets another. `None` when no stretch is marked, and no piece
/// when all are.
pub fn without_spans(sp: &SubPath, stretches: &[Span], remove: &[bool]) -> Option<Vec<SubPath>> {
    let n = stretches.len();
    let gone = |i: usize| remove.get(i).copied().unwrap_or(false);
    let first_gone = (0..n).find(|&i| gone(i))?;
    let nf = sp.segment_count() as f64;
    // A closed subpath is walked from its first removed stretch, so the stretches left on both
    // sides of its first anchor are one piece.
    let start = if sp.closed { first_gone } else { 0 };
    let mut out = vec![];
    let mut run: Option<Span> = None;
    for k in 0..n {
        let i = (start + k) % n;
        let Some(&(from, to)) = stretches.get(i).filter(|_| !gone(i)) else {
            out.extend(run.take().map(|r| slice(sp, r)));
            continue;
        };
        run = Some(match run {
            // Past the first anchor of a closed subpath the positions start again.
            Some((a, b)) => (a, if to < b { to + nf } else { to }),
            None => (from, to),
        });
    }
    out.extend(run.map(|r| slice(sp, r)));
    out.retain(|p| p.anchors.len() >= 2);
    Some(out)
}
