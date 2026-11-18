//! Lengthen an open path at one end, carrying on the way it runs there, or shorten it back along
//! itself: [`extend_open`], and [`extend_length_to`] for the length that brings the end nearest a
//! point (what a drag asks for).

use std::f64::consts::{FRAC_PI_2, PI};

use craft_geom::{Anchor, AnchorKind, SubPath};
use kurbo::{CubicBez, ParamCurve, ParamCurveArclen, Point, Vec2};

use super::{ACCURACY, TINY, nearest_on};
use crate::fit::end_tangent;

/// How an end carries on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtendMode {
    /// In a straight line along the end's direction.
    Straight,
    /// By stretching the last segment's curve on past its end (no anchor added).
    Bezier,
    /// Along the circle that matches the end's curvature, at most half way round.
    Arc,
}

impl ExtendMode {
    pub const ALL: [Self; 3] = [Self::Straight, Self::Bezier, Self::Arc];

    pub fn id(self) -> &'static str {
        match self {
            Self::Straight => "straight",
            Self::Bezier => "bezier",
            Self::Arc => "arc",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == s)
    }
}

/// The least length (pt) shortening leaves of a path.
const MIN_LEFT: f64 = 1e-3;
/// An end straighter than this radius (pt) carries on in a straight line in [`ExtendMode::Arc`].
const MAX_RADIUS: f64 = 1e6;
/// The farthest (in its parameter) [`ExtendMode::Bezier`] stretches a segment.
const MAX_STRETCH: f64 = 1024.0;

pub(super) fn finite(sp: &SubPath) -> bool {
    let ok = |p: Point| p.x.is_finite() && p.y.is_finite();
    sp.anchors.iter().all(|a| ok(a.p) && ok(a.h_in) && ok(a.h_out))
}

fn length(sp: &SubPath) -> f64 {
    (0..sp.segment_count()).map(|s| sp.segment(s).arclen(ACCURACY)).sum()
}

/// The last segment of an open subpath of two anchors or more.
fn last_segment(sp: &SubPath) -> Option<CubicBez> {
    let n = sp.segment_count();
    (n > 0).then(|| sp.segment(n - 1))
}

/// Open subpath `sp` (two anchors or more) lengthened by `length` pt at its first (`at_start`) or
/// last anchor the way `mode` says, or shortened along itself by a negative `length`. `None` when it
/// is closed, too short, not finite, or shortening would leave nothing of it.
pub fn extend_open(sp: &SubPath, at_start: bool, length: f64, mode: ExtendMode) -> Option<SubPath> {
    if sp.closed || sp.anchors.len() < 2 || !length.is_finite() || !finite(sp) {
        return None;
    }
    let mut s = sp.clone();
    if at_start {
        s.reverse();
    }
    let mut out = match mode {
        _ if length < 0.0 => trim_end(&s, -length)?,
        _ if length == 0.0 => s,
        ExtendMode::Straight => straight_end(s, length)?,
        ExtendMode::Bezier => bezier_end(s, length)?,
        ExtendMode::Arc => arc_end(s, length)?,
    };
    if at_start {
        out.reverse();
    }
    Some(out)
}

/// The length (pt) [`extend_open`] takes to bring the end of `sp` nearest `to`: positive along the
/// way `mode` carries the end on, negative back along the path (never all of it). `None` when there
/// is no such end.
pub fn extend_length_to(sp: &SubPath, at_start: bool, to: Point, mode: ExtendMode) -> Option<f64> {
    if !(to.x.is_finite() && to.y.is_finite()) {
        return None;
    }
    let mut s = sp.clone();
    if at_start {
        s.reverse();
    }
    let total = length(&s);
    let end = s.anchors.last()?.p;
    // Long enough to reach past the pointer; the original stays a prefix of the result.
    let probe = (to.distance(end) * 2.0).max(1.0);
    let ext = extend_open(&s, false, probe, mode)?;
    let (seg, t, _) = nearest_on(&ext, to)?;
    let before: f64 = (0..seg).map(|k| ext.segment(k).arclen(ACCURACY)).sum();
    let along = before + ext.segment(seg).subsegment(0.0..t).arclen(ACCURACY);
    Some((along - total).max(MIN_LEFT - total))
}

/// `sp` with `cut` pt taken off its end.
fn trim_end(sp: &SubPath, cut: f64) -> Option<SubPath> {
    if cut >= length(sp) - MIN_LEFT {
        return None;
    }
    let mut s = sp.clone();
    let mut left = cut;
    while left > 0.0 {
        let c = last_segment(&s)?;
        let len = c.arclen(ACCURACY);
        let line = s.segment_is_line(s.segment_count() - 1);
        if left >= len {
            s.anchors.pop();
            left -= len;
            if let Some(a) = s.anchors.last_mut() {
                a.h_out = a.p;
                a.kind = AnchorKind::Corner;
            }
            continue;
        }
        let part = c.subsegment(0.0..c.inv_arclen(len - left, ACCURACY));
        let k = s.anchors.len();
        if let Some(a) = s.anchors.get_mut(k - 2).filter(|_| !line) {
            a.h_out = part.p1;
        }
        if let Some(a) = s.anchors.last_mut() {
            *a = if line { Anchor::corner(part.p3) } else { Anchor { p: part.p3, h_in: part.p2, h_out: part.p3, kind: AnchorKind::Corner } };
        }
        break;
    }
    (s.anchors.len() >= 2).then_some(s)
}

/// `sp` with a straight segment `len` pt long added at its end, along its end's direction.
fn straight_end(mut s: SubPath, len: f64) -> Option<SubPath> {
    let dir = end_tangent(&last_segment(&s)?);
    let a = s.anchors.last_mut()?;
    a.h_out = a.p;
    let p = a.p + dir * len;
    s.anchors.push(Anchor::corner(p));
    Some(s)
}

/// `sp` with its last segment's curve stretched on by `len` pt (its end anchor moved, no anchor
/// added); a straight last segment just grows.
fn bezier_end(mut s: SubPath, len: f64) -> Option<SubPath> {
    let c = last_segment(&s)?;
    let k = s.anchors.len();
    if s.segment_is_line(k - 2) {
        let p = c.p3 + end_tangent(&c) * len;
        *s.anchors.last_mut()? = Anchor::corner(p);
        return Some(s);
    }
    let target = c.arclen(ACCURACY) + len;
    let reach = |t: f64| c.subsegment(0.0..t).arclen(ACCURACY);
    let mut hi = 2.0;
    while reach(hi) < target && hi < MAX_STRETCH {
        hi *= 2.0;
    }
    let mut lo = 1.0;
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if reach(mid) < target { lo = mid } else { hi = mid }
    }
    let ext = c.subsegment(0.0..hi);
    s.anchors.get_mut(k - 2)?.h_out = ext.p1;
    *s.anchors.last_mut()? = Anchor { p: ext.p3, h_in: ext.p2, h_out: ext.p3, kind: AnchorKind::Corner };
    Some(s)
}

/// The signed radius of the circle through three points (positive when the turn from `a` through
/// `b` to `c` is counter-clockwise in the x→y sense); `None` when they lie in a line.
fn signed_radius(a: Point, b: Point, c: Point) -> Option<f64> {
    let cross = (b - a).cross(c - b);
    let r = (b - a).hypot() * (c - b).hypot() * (c - a).hypot() / (2.0 * cross.abs());
    (cross.abs() > TINY && r.is_finite()).then(|| r.copysign(cross))
}

/// `v` turned by `a` radians (x toward y).
fn rot(v: Vec2, a: f64) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// `sp` carried on by an arc `len` pt long (at most half a circle) of the circle that leaves its
/// end in its direction with the curvature it ends with; straight where it ends straight.
fn arc_end(mut s: SubPath, len: f64) -> Option<SubPath> {
    let c = last_segment(&s)?;
    let dir = end_tangent(&c);
    let Some(r) = signed_radius(c.eval(0.96), c.eval(0.98), c.p3).filter(|r| r.abs() <= MAX_RADIUS) else { return straight_end(s, len) };
    let radius = r.abs();
    let turn = (len / radius).min(PI).copysign(r);
    let normal: Vec2 = Vec2::new(-dir.y, dir.x) * r.signum();
    let p = c.p3;
    let centre = p + normal * radius;
    let pieces = (turn.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = turn / pieces as f64;
    let h = 4.0 / 3.0 * (step.abs() / 4.0).tan() * radius;
    let at = |phi: f64| (centre + rot(p - centre, phi), rot(dir, phi));
    let first = at(0.0);
    let end = s.anchors.last_mut()?;
    end.h_out = first.0 + first.1 * h;
    if end.has_in() {
        end.kind = AnchorKind::Smooth;
    }
    for j in 1..=pieces {
        let (q, t) = at(step * j as f64);
        let h_out = if j < pieces { q + t * h } else { q };
        let kind = if j < pieces { AnchorKind::Smooth } else { AnchorKind::Corner };
        s.anchors.push(Anchor { p: q, h_in: q - t * h, h_out, kind });
    }
    Some(s)
}
