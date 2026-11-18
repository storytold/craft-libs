//! Path Precision Tools' geometry: anchor edits for the Anchor Inspector (smooth by the auto-smooth
//! rule, equalise handles, add evenly spaced anchors, close), what its live path effects add to them
//! ([`add_anchors`], [`smooth_anchors`], [`split_at_anchors`]), the readouts the Measure & Inspect
//! tool shows over a path, open paths lengthened or shortened at an end ([`extend`]), the places
//! where paths cross ([`crossings`]), the stretches between them ([`spans`]), anchors slid along
//! their path ([`slide`]), touching edges merged ([`shared_edge`]), paths divided into dashes
//! ([`divide_dashes`]), two circles joined by a smooth bridge ([`smooth_bridge`]), the common
//! tangents of two convex outlines ([`common_tangents`]), the
//! Parametric Shape Tool's shapes ([`shapes`]) and the circles that touch two or three shapes
//! ([`tangent_circles`]).

mod bridge;
mod crossings;
mod dashes;
mod extend;
pub mod shapes;
mod shared_edge;
mod slide;
mod tangent_circle;
mod tangents;
#[cfg(test)]
mod tests;
mod trim;

use std::collections::BTreeSet;

use craft_geom::{Anchor, AnchorKind, PathData, SubPath};
use kurbo::{CubicBez, ParamCurve, ParamCurveArclen, ParamCurveNearest, Point, Vec2};

pub use bridge::{BridgeError, circle_of, smooth_bridge};
pub use crossings::{MAX_CROSSED_BY_SEGMENTS, MAX_CROSSING_SEGMENTS, Place, Work, crossings, crossings_on, crossings_on_within, insert_at};
pub use dashes::{DashSpec, Piece, divide_dashes};
pub use extend::{ExtendMode, extend_length_to, extend_open};
pub use shared_edge::{
    COINCIDENCE, EndSide, Follow, MAX_COINCIDENCE, MergePlan, MergePreview, Stretch, apply_merge, ends_at, merge_preview, stretches_at,
};
pub use slide::{MAX_SLIDE, slide_anchor, slide_stops, slide_t_at};
pub use tangent_circle::{Circle, MAX_TANGENT_CIRCLE_SEGMENTS, MAX_TANGENT_CIRCLES, TangentCircleError, tangent_circle_segments, tangent_circles};
pub use tangents::{Coincide, MAX_TANGENT_SEGMENTS, Outline, OutlineError, Tangent, TangentKind, common_tangents};
pub use trim::{Span, slice, span_at, spans, without_spans};

use crate::fit::{end_tangent, start_tangent};

/// The auto-smooth rule's handle length as a share of the nearer neighbour's distance: the share
/// that makes a square's four corners a circle.
pub const SMOOTH_RATIO: f64 = 0.3905;
/// The smallest and largest smoothing ratio.
pub const SMOOTH_RATIO_RANGE: (f64, f64) = (0.1, 0.5);
/// Most anchors [`add_points`] puts on one segment.
pub const MAX_ADDED_PER_SEGMENT: usize = 100;

/// Arc-length accuracy (pt) of the measurements here.
const ACCURACY: f64 = 1e-6;
/// Shorter than this (pt) is no handle and no distance.
const TINY: f64 = 1e-9;

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l > TINY && l.is_finite()).then(|| v / l)
}

/// The indices of anchor `i`'s neighbours in `sp` (`None` past an open end).
pub(crate) fn neighbours(sp: &SubPath, i: usize) -> (Option<usize>, Option<usize>) {
    let n = sp.anchors.len();
    let prev = if i > 0 { Some(i - 1) } else { (sp.closed && n > 1).then(|| n - 1) };
    let next = if i + 1 < n { Some(i + 1) } else { (sp.closed && n > 1).then_some(0) };
    (prev.filter(|&j| j != i), next.filter(|&j| j != i))
}

/// Make anchor `i` smooth by the auto-smooth rule: a lone handle is mirrored at its length; with
/// none, both handles stand at right angles to the bisector of the angle its neighbours make at
/// it, each `ratio` × the nearer neighbour's distance long; two handles are brought into line,
/// keeping their lengths. An open end without handles gets one aimed at its neighbour's facing
/// handle. False when nothing changes or there is no direction to take.
pub fn auto_smooth(sp: &mut SubPath, i: usize, ratio: f64) -> bool {
    let (prev, next) = neighbours(sp, i);
    let at = |j: Option<usize>| j.and_then(|j| sp.anchors.get(j)).copied();
    let (prev, next) = (at(prev), at(next));
    let Some(a) = sp.anchors.get_mut(i) else { return false };
    let before = *a;
    let p = a.p;
    match (a.has_in(), a.has_out()) {
        (true, true) => {
            let (Some(o), Some(n)) = (unit(a.h_out - p), unit(p - a.h_in)) else { return false };
            // Already in line: only the kind may change.
            if o.cross(n).abs() <= 1e-12 && o.dot(n) > 0.0 {
                a.kind = AnchorKind::Smooth;
                return *a != before;
            }
            let Some(d) = unit(o + n) else { return false };
            a.h_out = p + d * (a.h_out - p).hypot();
            a.h_in = p - d * (a.h_in - p).hypot();
        }
        (true, false) if next.is_some() => a.h_out = p - (a.h_in - p),
        (false, true) if prev.is_some() => a.h_in = p - (a.h_out - p),
        (false, false) => match (prev, next) {
            (Some(pr), Some(nx)) => {
                let (dp, dn) = (pr.p.distance(p), nx.p.distance(p));
                let (Some(up), Some(un)) = (unit(pr.p - p), unit(nx.p - p)) else { return false };
                let Some(dir) = unit(un - up) else { return false };
                let len = dp.min(dn) * ratio;
                a.h_out = p + dir * len;
                a.h_in = p - dir * len;
            }
            // An open end: aim at the neighbour's facing handle (or the neighbour itself).
            (None, Some(nx)) => {
                let Some(dir) = unit(nx.h_in - p) else { return false };
                a.h_out = p + dir * (nx.p.distance(p) * ratio);
            }
            (Some(pr), None) => {
                let Some(dir) = unit(pr.h_out - p) else { return false };
                a.h_in = p + dir * (pr.p.distance(p) * ratio);
            }
            (None, None) => return false,
        },
        _ => return false,
    }
    a.kind = if a.has_in() && a.has_out() { AnchorKind::Smooth } else { AnchorKind::Corner };
    *a != before
}

/// Make anchor `i` a corner: its handles retracted with `retract`, else left where they are.
/// False when nothing changes.
pub fn make_corner(sp: &mut SubPath, i: usize, retract: bool) -> bool {
    let Some(a) = sp.anchors.get_mut(i) else { return false };
    let before = *a;
    if retract {
        a.retract();
    }
    a.kind = AnchorKind::Corner;
    *a != before
}

/// Give anchor `i`'s two handles the same length, their average, each keeping its direction; a
/// missing one (where there is a segment for it) is made opposite the other at its length. False
/// when there is nothing to equalise.
pub fn equalize_handles(sp: &mut SubPath, i: usize) -> bool {
    let (prev, next) = neighbours(sp, i);
    let Some(a) = sp.anchors.get_mut(i) else { return false };
    let before = *a;
    let p = a.p;
    match (a.has_in(), a.has_out()) {
        (true, true) => {
            let len = ((a.h_in - p).hypot() + (a.h_out - p).hypot()) / 2.0;
            let (Some(di), Some(dout)) = (unit(a.h_in - p), unit(a.h_out - p)) else { return false };
            a.h_in = p + di * len;
            a.h_out = p + dout * len;
        }
        (true, false) if next.is_some() => a.h_out = p - (a.h_in - p),
        (false, true) if prev.is_some() => a.h_in = p - (a.h_out - p),
        _ => return false,
    }
    *a != before
}

/// `sp` with `count` anchors added on each of `segments` (all of them when `None`), splitting each
/// into `count + 1` pieces of equal length; the shape doesn't change. → (the subpath, the indexes of
/// the anchors added).
pub fn add_points(sp: &SubPath, segments: Option<&BTreeSet<usize>>, count: usize) -> (SubPath, BTreeSet<usize>) {
    let count = count.min(MAX_ADDED_PER_SEGMENT);
    let mut hits = vec![];
    for seg in (0..sp.segment_count()).filter(|s| segments.is_none_or(|set| set.contains(s))) {
        let c = sp.segment(seg);
        let len = c.arclen(ACCURACY);
        if !(len > TINY && len.is_finite()) {
            continue;
        }
        hits.extend((1..=count).map(|k| (seg, c.inv_arclen(len * k as f64 / (count + 1) as f64, ACCURACY))));
    }
    insert_at(sp, &hits)
}

/// The segments [`add_anchors`] adds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    All,
    Curved,
    Straight,
}

/// How [`add_anchors`] spaces its anchors along a segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnchorSpacing {
    /// This many, evenly spaced by length.
    Count(usize),
    /// One every this many points from the segment's start, none on its end.
    Distance(f64),
}

/// `sp` with anchors added on the segments `which` picks, spaced as `spacing` says and at most `cap`
/// on one segment; the shape doesn't change. `None` when that would add more than `room` anchors.
pub fn add_anchors(sp: &SubPath, which: SegmentKind, spacing: AnchorSpacing, cap: usize, mut room: usize) -> Option<SubPath> {
    let mut hits = vec![];
    for seg in 0..sp.segment_count() {
        let picked = match which {
            SegmentKind::All => true,
            SegmentKind::Curved => !sp.segment_is_line(seg),
            SegmentKind::Straight => sp.segment_is_line(seg),
        };
        let c = sp.segment(seg);
        let len = c.arclen(ACCURACY);
        if !(picked && len > TINY && len.is_finite()) {
            continue;
        }
        let (count, step) = match spacing {
            AnchorSpacing::Count(n) => (n, len / (n as f64 + 1.0)),
            AnchorSpacing::Distance(d) if d > 0.0 => (((len / d).ceil() as usize).saturating_sub(1), d),
            AnchorSpacing::Distance(_) => (0, 0.0),
        };
        let count = count.min(cap);
        room = room.checked_sub(count)?;
        hits.extend((1..=count).map(|k| (seg, c.inv_arclen(step * k as f64, ACCURACY))));
    }
    Some(insert_at(sp, &hits).0)
}

/// Make every anchor of `sp` smooth by the auto-smooth rule ([`auto_smooth`]). An open path's ends
/// come last, so each aims at the handle its neighbour ended up with, whichever way the path runs.
pub fn smooth_anchors(sp: &mut SubPath, ratio: f64) {
    let (n, closed) = (sp.anchors.len(), sp.closed);
    let ends = if closed { vec![] } else { vec![0, n.saturating_sub(1)] };
    for i in (0..n).filter(|&i| closed || (i > 0 && i + 1 < n)).chain(ends) {
        auto_smooth(sp, i, ratio);
    }
}

/// `sp` cut at its `corners`, its `smooth` anchors, or both ([`SubPath::cut_at`]); the shape doesn't
/// change. An anchor is smooth when its handles stand in line, as the outline shows it, whatever
/// kind it was given.
pub fn split_at_anchors(sp: &SubPath, corners: bool, smooth: bool) -> Vec<SubPath> {
    let is_smooth = |a: &Anchor| Anchor::with_handles(a.p, a.h_in, a.h_out).kind == AnchorKind::Smooth;
    let at = sp.anchors.iter().enumerate().filter(|(_, a)| if is_smooth(a) { smooth } else { corners }).map(|(i, _)| i).collect();
    sp.cut_at(&at)
}

/// How [`close`] joins an open subpath's ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    /// With the handles the ends already have.
    Handles,
    /// With a straight segment.
    Straight,
    /// With a segment that leaves both ends smoothly ([`auto_smooth`]).
    Smooth,
}

impl CloseMode {
    pub const ALL: [Self; 3] = [Self::Handles, Self::Straight, Self::Smooth];

    pub fn id(self) -> &'static str {
        match self {
            Self::Handles => "handles",
            Self::Straight => "straight",
            Self::Smooth => "smooth",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == s)
    }
}

/// Close open subpath `sp` (of two anchors or more) the way `mode` says. False when it is closed
/// already or too short.
pub fn close(sp: &mut SubPath, mode: CloseMode, ratio: f64) -> bool {
    if sp.closed || sp.anchors.len() < 2 {
        return false;
    }
    if mode == CloseMode::Straight {
        if let Some(a) = sp.anchors.first_mut() {
            a.h_in = a.p;
        }
        if let Some(a) = sp.anchors.last_mut() {
            a.h_out = a.p;
        }
    }
    sp.closed = true;
    if mode == CloseMode::Smooth {
        let last = sp.anchors.len() - 1;
        auto_smooth(sp, 0, ratio);
        auto_smooth(sp, last, ratio);
    }
    true
}

/// A handle's length (pt) and direction (degrees counter-clockwise on the page from the x axis);
/// `None` for no handle.
pub fn handle_polar(p: Point, h: Point) -> Option<(f64, f64)> {
    let v = h - p;
    let len = v.hypot();
    (len > TINY).then(|| (len, angle_of(v)))
}

/// Direction of `v` in degrees counter-clockwise on the page (y points down), in (-180, 180].
pub fn angle_of(v: Vec2) -> f64 {
    (-v.y).atan2(v.x).to_degrees() + 0.0
}

/// Where a handle `len` long at `deg` degrees (counter-clockwise on the page) from `p` ends.
pub fn handle_at(p: Point, len: f64, deg: f64) -> Point {
    let (s, c) = deg.to_radians().sin_cos();
    p + Vec2::new(c, -s) * len
}

/// What the Measure & Inspect tool reads off a path at a point near it.
#[derive(Clone, Debug, PartialEq)]
pub struct Inspection {
    pub subpath: usize,
    pub closed: bool,
    pub anchors: usize,
    /// The subpath's length (pt).
    pub length: f64,
    /// The area the subpath encloses (pt², an open one closed by a straight line).
    pub area: f64,
    pub segment: usize,
    pub t: f64,
    /// The point on the path nearest the pointer.
    pub point: Point,
    pub segment_length: f64,
    /// The direction the path runs there, in degrees counter-clockwise on the page.
    pub tangent: f64,
    /// The radius of curvature there (pt); `None` where the path runs straight.
    pub radius: Option<f64>,
    /// The anchor near the pointer, when one is.
    pub anchor: Option<AnchorInspection>,
}

/// An anchor's readouts.
#[derive(Clone, Debug, PartialEq)]
pub struct AnchorInspection {
    pub index: usize,
    pub p: Point,
    pub smooth: bool,
    /// The angle the path makes at the anchor (degrees: 90 at a square's corner, 180 where it runs
    /// straight on); `None` at an open end.
    pub angle: Option<f64>,
    /// Each handle's (length, direction): `None` for no handle.
    pub handle_in: Option<(f64, f64)>,
    pub handle_out: Option<(f64, f64)>,
}

/// Radius of curvature of `c` at `t`: `None` where it runs straight or has no direction.
fn radius_at(c: &CubicBez, t: f64) -> Option<f64> {
    use kurbo::ParamCurveDeriv;
    let d1 = c.deriv();
    let d = d1.eval(t).to_vec2();
    let dd = d1.deriv().eval(t).to_vec2();
    let speed = d.hypot();
    let k = d.cross(dd) / (speed * speed * speed);
    (speed > TINY && k.is_finite() && k.abs() > 1e-9).then(|| 1.0 / k.abs())
}

/// The direction `c` runs at `t` (unit), falling back to its chord's ends where the derivative
/// vanishes (a retracted handle).
fn tangent_at(c: &CubicBez, t: f64) -> Vec2 {
    use kurbo::ParamCurveDeriv;
    unit(c.deriv().eval(t).to_vec2()).unwrap_or_else(|| if t < 0.5 { start_tangent(c) } else { end_tangent(c) })
}

/// The readouts of anchor `i` of `sp`.
pub fn inspect_anchor(sp: &SubPath, i: usize) -> Option<AnchorInspection> {
    let a = sp.anchors.get(i)?;
    let (prev, next) = neighbours(sp, i);
    let angle = match (prev, next) {
        (Some(pr), Some(_)) if sp.segment_count() > 0 => {
            let (into, out) = (end_tangent(&sp.segment(pr)), start_tangent(&sp.segment(i)));
            Some((-into).dot(out).clamp(-1.0, 1.0).acos().to_degrees())
        }
        _ => None,
    };
    Some(AnchorInspection {
        index: i,
        p: a.p,
        smooth: a.kind == AnchorKind::Smooth,
        angle,
        handle_in: handle_polar(a.p, a.h_in),
        handle_out: handle_polar(a.p, a.h_out),
    })
}

/// What `path` reads at `p`: its point nearest `p` and the readouts there, and the anchor within
/// `tol` of `p` if one is. `None` for a path with no segments or one with a non-finite coordinate.
pub fn inspect(path: &PathData, p: Point, tol: f64) -> Option<Inspection> {
    let finite = |q: Point| q.x.is_finite() && q.y.is_finite();
    if !finite(p) || !path.anchors().all(|(_, _, a)| finite(a.p) && finite(a.h_in) && finite(a.h_out)) {
        return None;
    }
    let (si, seg, t, point, _) = path.nearest(p)?;
    let sp = path.subpaths.get(si)?;
    let c = sp.segment(seg);
    let anchor = [(seg, c.p0), ((seg + 1) % sp.anchors.len(), c.p3)]
        .into_iter()
        .filter(|(_, q)| q.distance(p) <= tol)
        .min_by(|a, b| a.1.distance(p).total_cmp(&b.1.distance(p)))
        .and_then(|(i, _)| inspect_anchor(sp, i));
    Some(Inspection {
        subpath: si,
        closed: sp.closed,
        anchors: sp.anchors.len(),
        length: (0..sp.segment_count()).map(|s| sp.segment(s).arclen(ACCURACY)).sum(),
        area: sp.signed_area().abs(),
        segment: seg,
        t,
        point,
        segment_length: c.arclen(ACCURACY),
        tangent: angle_of(tangent_at(&c, t)),
        radius: radius_at(&c, t),
        anchor,
    })
}

/// The point of `sp`'s segments nearest `p`: (segment, t, distance).
pub(crate) fn nearest_on(sp: &SubPath, p: Point) -> Option<(usize, f64, f64)> {
    (0..sp.segment_count())
        .map(|s| {
            let n = sp.segment(s).nearest(p, ACCURACY);
            (s, n.t, n.distance_sq.sqrt())
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}
