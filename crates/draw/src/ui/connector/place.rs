//! Where a connector end lands, shared by the tool and the handles: the end under a point, the route
//! of a connector through its targets, the bends that can be grabbed and the next object an arrow
//! key moves an end to.

use craft_behavior::CanvasView;
use craft_keymap::Mods;
use kurbo::Point;

use super::{ConnectionTarget, ConnectorEnd, ConnectorEndKind, ConnectorItem};
use crate::geometry::connector::{ConnectionSide, RouteEnd, Routing, path_handles, route_along};

/// The shortest elbow run, in screen points, that has a bend handle.
pub(crate) const MIN_RUN: f32 = 28.0;

/// The outer part of a box where a point chooses a side, as a fraction of the half-size
/// (FigCraft's `side_at`: the inner half of the box chooses none).
const INNER: f64 = 0.5;

/// A divisor floor for a box with no width or height.
const TINY: f64 = 1e-6;

/// What is at a point: the end a connector would have there.
#[derive(Clone, Debug)]
pub(crate) struct Placed {
    pub(crate) end: ConnectorEnd,
    pub(crate) route: RouteEnd,
    /// The target under the point (its sites show), whether or not the end glues to it.
    pub(crate) target: Option<u64>,
    /// The site the end glues to, as its index on the target.
    pub(crate) active: Option<u32>,
}

/// The side of box `b` a connector end at `p` chooses (FigCraft #1708): the one `p` is towards
/// anywhere in the box's outer half, whatever its proportions, so a whole edge picks its side. In
/// the middle of the box none is chosen ([`ConnectionSide::Auto`]).
fn side_at(b: kurbo::Rect, p: Point) -> ConnectionSide {
    let c = b.center();
    let (x, y) = ((p.x - c.x) / (b.width() / 2.0).max(TINY), (p.y - c.y) / (b.height() / 2.0).max(TINY));
    if x.abs().max(y.abs()) < INNER {
        ConnectionSide::Auto
    } else if x.abs() >= y.abs() {
        if x > 0.0 { ConnectionSide::Right } else { ConnectionSide::Left }
    } else if y > 0.0 {
        ConnectionSide::Bottom
    } else {
        ConnectionSide::Top
    }
}

/// The index of `side` among the default sites.
fn side_index(side: ConnectionSide) -> Option<u32> {
    ConnectionSide::ALL.iter().position(|&s| s == side).and_then(|i| u32::try_from(i).ok())
}

/// The object `end` is glued to.
pub(crate) fn glued(end: &ConnectorEnd) -> Option<u64> {
    match end {
        ConnectorEnd::Glued { target, .. } => Some(*target),
        ConnectorEnd::Free(_) => None,
    }
}

/// Where a route end sits: the point, or the middle of the named side (the middle of the box for
/// `Auto`).
pub(crate) fn anchor(end: RouteEnd) -> Point {
    match end {
        RouteEnd::Point(p) | RouteEnd::On(_, _, p) => p,
        RouteEnd::Box(b, side) => side.point(b),
    }
}

/// The end at `at`: glued to the nearest site of the topmost target containing `at` within `reach`
/// screen points, a target being hit within `reach` of its bounds, else the side the point is towards, else free. With ⌘ or Ctrl held, a target with
/// no sites takes the point itself.
pub(crate) fn end_at(targets: &[ConnectionTarget], at: Point, view: &CanvasView, reach: f32, mods: Mods) -> Placed {
    // inv-figcraft: crates/engine/src/boards/board/tools.rs:98-124 (the side within `ATTACH_PX`
    // :12, else the side the pointer is towards `side_at`, ⌘ glues at the point).
    // inv-deckcraft: crates/engine/src/connect.rs:1-45 `sites`, `nearest_site` (a target with
    // numbered sites glues only within the radius of one).
    let free = Placed { end: ConnectorEnd::Free(at), route: RouteEnd::Point(at), target: None, active: None };
    let reach = view.doc_len(reach);
    let slack = reach;
    let Some(t) = targets.iter().find(|t| t.bounds.abs().inflate(slack, slack).contains(at)) else { return free };
    let b = t.bounds.abs();
    let target = Some(t.id);
    let dist = |p: Point| (p - at).hypot();
    if !t.sites.is_empty() {
        let nearest = t.sites.iter().filter(|s| dist(s.point) <= reach).min_by(|x, y| dist(x.point).total_cmp(&dist(y.point)));
        return match nearest {
            Some(s) => Placed {
                end: ConnectorEnd::Glued { target: t.id, side: s.side, site: Some(s.index), at: None },
                route: RouteEnd::On(b, s.side, s.point),
                target,
                active: Some(s.index),
            },
            None => Placed { target, ..free },
        };
    }
    if (mods.cmd || mods.ctrl) && b.width() > 0.0 && b.height() > 0.0 {
        let frac = [((at.x - b.x0) / b.width()).clamp(0.0, 1.0), ((at.y - b.y0) / b.height()).clamp(0.0, 1.0)];
        let p = Point::new(b.x0 + frac[0] * b.width(), b.y0 + frac[1] * b.height());
        return Placed {
            end: ConnectorEnd::Glued { target: t.id, side: ConnectionSide::Auto, site: None, at: Some(frac) },
            route: RouteEnd::On(b, ConnectionSide::Auto, p),
            target,
            active: None,
        };
    }
    let near = ConnectionSide::ALL.iter().map(|&s| (s, dist(s.point(b)))).filter(|&(_, d)| d <= reach).min_by(|x, y| x.1.total_cmp(&y.1));
    let side = near.map_or_else(|| side_at(b, at), |(s, _)| s);
    Placed { end: ConnectorEnd::Glued { target: t.id, side, site: None, at: None }, route: RouteEnd::Box(b, side), target, active: side_index(side) }
}

/// `end` as the geometry routes it; `None` when its object is not among the targets.
pub(crate) fn route_end(targets: &[ConnectionTarget], end: &ConnectorEnd) -> Option<RouteEnd> {
    match *end {
        ConnectorEnd::Free(p) => Some(RouteEnd::Point(p)),
        ConnectorEnd::Glued { target, side, site, at } => {
            let t = targets.iter().find(|t| t.id == target)?;
            let b = t.bounds.abs();
            if let Some(f) = at {
                return Some(RouteEnd::On(b, side, Point::new(b.x0 + f[0] * b.width(), b.y0 + f[1] * b.height())));
            }
            match site.and_then(|i| t.side_sites().into_iter().find(|s| s.index == i)) {
                Some(s) => Some(RouteEnd::On(b, side, s.point)),
                None => Some(RouteEnd::Box(b, side)),
            }
        }
    }
}

/// A connector's ends as routed and its points; `None` when an end's object is not among the targets.
pub(crate) fn geometry(targets: &[ConnectionTarget], item: &ConnectorItem) -> Option<(RouteEnd, RouteEnd, Vec<Point>)> {
    let (from, to) = (route_end(targets, &item.from)?, route_end(targets, &item.to)?);
    let points = route_along(from, to, item.routing, item.path.as_ref())?;
    Some((from, to, points))
}

/// The bends of the route through `points` that are drawn and can be grabbed at `view`'s zoom:
/// [`path_handles`] without the elbow runs too short on screen to hold one. The portion is the index
/// [`reshaped`](crate::geometry::connector::reshaped) takes.
pub(crate) fn visible_bends(points: &[Point], routing: Routing, view: &CanvasView) -> Vec<(usize, Point)> {
    let long = |i: usize| match (points.get(i), points.get(i + 1)) {
        (Some(&a), Some(&b)) => view.to_screen(a).distance(view.to_screen(b)) >= MIN_RUN,
        _ => false,
    };
    path_handles(points, routing).into_iter().filter(|&(i, _)| routing == Routing::Curved || long(i)).collect()
}

/// The end `end` of `item` moved to the next (or previous) target in the targets' order, skipping the
/// object the other end is on.
pub(crate) fn stepped_end(targets: &[ConnectionTarget], item: &ConnectorItem, end: ConnectorEndKind, forward: bool) -> Option<ConnectorEnd> {
    let (mine, other) = if end == ConnectorEndKind::Start { (&item.from, &item.to) } else { (&item.to, &item.from) };
    let skip = glued(other);
    let candidates: Vec<&ConnectionTarget> = targets.iter().filter(|t| Some(t.id) != skip).collect();
    let now = glued(mine).and_then(|id| candidates.iter().position(|t| t.id == id));
    let n = candidates.len();
    let next = match (now, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, true) => 0,
        (None, false) => n.checked_sub(1)?,
    };
    let t = candidates.get(next)?;
    Some(match t.sites.first() {
        Some(s) => ConnectorEnd::Glued { target: t.id, side: s.side, site: Some(s.index), at: None },
        None => ConnectorEnd::Glued { target: t.id, side: ConnectionSide::Auto, site: None, at: None },
    })
}
