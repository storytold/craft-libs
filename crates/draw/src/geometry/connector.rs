//! Connector routes between objects: [`route`], [`route_along`], [`attached_sides`],
//! [`path_handles`] and [`reshaped`], ported from FigCraft's `board_route` (no egui). The
//! Connector tool previews with them (`craft_draw::ui::connector`), and an app's engine routes the
//! connectors it stores with the same functions, so the two never disagree.
//!
//! A connector's route is document geometry: [`ROOM`] is a constant, not a theme token, so a
//! theme never moves a connector.

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

/// How far an elbow route going around the objects it joins keeps from them, document units
/// (FigCraft's `ROOM`).
pub const ROOM: f64 = 24.0;

/// How a connector runs between its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Routing {
    /// A direct line between the ends.
    #[default]
    Straight,
    /// Right angles only: out of each end along its side's normal, turning once or twice, around
    /// the objects it joins when the short way would cross them.
    Elbow,
    /// One smooth curve, leaving and arriving along the sides' normals.
    Curved,
}

/// The side of an object a connector end leaves by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionSide {
    /// The side facing the other end.
    #[default]
    Auto,
    #[allow(missing_docs)]
    Top,
    #[allow(missing_docs)]
    Right,
    #[allow(missing_docs)]
    Bottom,
    #[allow(missing_docs)]
    Left,
}

impl ConnectionSide {
    /// The four sides clockwise from the top, the default connection sites' order.
    pub const ALL: [ConnectionSide; 4] = [ConnectionSide::Top, ConnectionSide::Right, ConnectionSide::Bottom, ConnectionSide::Left];

    /// The side's middle on `bounds`; the centre for [`Auto`](Self::Auto).
    pub fn point(self, bounds: Rect) -> Point {
        let c = bounds.center();
        match self {
            ConnectionSide::Top => Point::new(c.x, bounds.y0),
            ConnectionSide::Right => Point::new(bounds.x1, c.y),
            ConnectionSide::Bottom => Point::new(c.x, bounds.y1),
            ConnectionSide::Left => Point::new(bounds.x0, c.y),
            ConnectionSide::Auto => c,
        }
    }

    /// Left or right.
    pub fn horizontal(self) -> bool {
        matches!(self, ConnectionSide::Left | ConnectionSide::Right)
    }

    /// The way out of the side (y down); zero for [`Auto`](Self::Auto).
    pub fn normal(self) -> Vec2 {
        match self {
            ConnectionSide::Top => Vec2::new(0.0, -1.0),
            ConnectionSide::Right => Vec2::new(1.0, 0.0),
            ConnectionSide::Bottom => Vec2::new(0.0, 1.0),
            ConnectionSide::Left => Vec2::new(-1.0, 0.0),
            ConnectionSide::Auto => Vec2::ZERO,
        }
    }
}

/// A box edge counts as outside: a run along or out of an edge is not inside, by this much
/// (FigCraft's `EDGE`, document units).
const EDGE: f64 = 0.5;

/// A path of more runs than this can't be made by hand ([`reshaped`] refuses another).
const MAX_RUNS: usize = 64;

/// Two control points moved by `d` move a cubic's middle by 3/4 of `d`, so a bow is applied as
/// `d * 4/3`.
const BOW_GAIN: f64 = 4.0 / 3.0;

/// A divisor floor that keeps a zero-length division finite.
const TINY: f64 = 1e-9;

/// One end of a route.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteEnd {
    /// A free end at a point.
    Point(Point),
    /// Glued to the middle of a side of the box (or the side facing the other end).
    Box(Rect, ConnectionSide),
    /// Glued to a point on the object in the box, placed by hand or a numbered site, leaving by the
    /// side given.
    On(Rect, ConnectionSide, Point),
}

/// A connector's path as reshaped by hand, measured from the middle of its two ends so the shape
/// travels with what it joins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorPath {
    /// An elbow's straight runs, each at right angles to the one before: whether the first runs
    /// across (else up or down), and where the runs between the first and last lie (x of one that
    /// runs up or down, y of one that runs across). The first and last lie on the ends.
    Runs {
        /// Whether the first run goes across (else up or down).
        across: bool,
        /// Positions of the runs between the first and last, in document units.
        at: Vec<f64>,
    },
    /// How far a curve's middle was moved.
    Bow {
        /// Horizontal move of the curve's middle, in document units.
        x: f64,
        /// Vertical move of the curve's middle, in document units.
        y: f64,
    },
}

impl RouteEnd {
    fn centre(self) -> Point {
        match self {
            RouteEnd::Point(p) | RouteEnd::On(_, _, p) => p,
            RouteEnd::Box(b, _) => b.center(),
        }
    }
}

/// The points a connector runs through from `from` to `to`: from the middle of the side of each
/// box facing the other end (or the side it names), straight, in right angles keeping [`ROOM`]
/// from the joined boxes, or (curved) its two ends with the curve's two control points between
/// them. `None` when a point isn't finite.
pub fn route(from: RouteEnd, to: RouteEnd, routing: Routing) -> Option<Vec<Point>> {
    let (p0, s0) = attach(from, to.centre());
    let (p1, s1) = attach(to, from.centre());
    let boxes: Vec<Rect> = [from, to].into_iter().filter_map(|e| if let RouteEnd::Box(b, _) = e { Some(b) } else { None }).collect();
    let pts = match routing {
        Routing::Straight => tidy(vec![p0, p1]),
        Routing::Elbow => elbow(p0, s0, p1, s1, &boxes),
        Routing::Curved => curve(p0, s0, p1, s1),
    };
    finite(pts)
}

/// [`route`] for a connector whose `path` may have been reshaped by hand; a path that isn't for
/// its `routing` is ignored.
pub fn route_along(from: RouteEnd, to: RouteEnd, routing: Routing, path: Option<&ConnectorPath>) -> Option<Vec<Point>> {
    let (p0, s0) = attach(from, to.centre());
    let (p1, s1) = attach(to, from.centre());
    let pts = match (routing, path) {
        (Routing::Elbow, Some(ConnectorPath::Runs { across, at })) => along_runs(p0, p1, *across, at),
        (Routing::Curved, Some(ConnectorPath::Bow { x, y })) => {
            let d = Vec2::new(*x, *y) * BOW_GAIN;
            let pull = |(i, p): (usize, Point)| if i == 1 || i == 2 { p + d } else { p };
            curve(p0, s0, p1, s1).into_iter().enumerate().map(pull).collect()
        }
        _ => return route(from, to, routing),
    };
    finite(pts)
}

/// The sides a connector's ends leave by as routed now; `None` for a free end.
pub fn attached_sides(from: RouteEnd, to: RouteEnd) -> (Option<ConnectionSide>, Option<ConnectionSide>) {
    (attach(from, to.centre()).1, attach(to, from.centre()).1)
}

/// The route's path handles: one per run of an elbow and one for a curve, each with the portion
/// of the path it moves and where it sits. None for a straight route.
pub fn path_handles(points: &[Point], routing: Routing) -> Vec<(usize, Point)> {
    match routing {
        Routing::Straight => Vec::new(),
        Routing::Curved => curve_middle(points).map(|m| (0, m)).into_iter().collect(),
        Routing::Elbow => points.windows(2).enumerate().map(|(i, w)| (i, w[0].midpoint(w[1]))).collect(),
    }
}

/// The path after dragging `portion`'s handle to `at`: an elbow's run moves there, and a first or
/// last run stays on its end through an extra run; a curve's middle moves there. `None` for a
/// straight connector, a portion it doesn't have, or a path of too many runs (64).
pub fn reshaped(from: RouteEnd, to: RouteEnd, routing: Routing, path: Option<&ConnectorPath>, portion: usize, at: Point) -> Option<ConnectorPath> {
    match routing {
        Routing::Straight => None,
        Routing::Curved => {
            let m = curve_middle(&route(from, to, routing)?)?;
            Some(ConnectorPath::Bow { x: at.x - m.x, y: at.y - m.y })
        }
        Routing::Elbow => reshaped_runs(&route_along(from, to, routing, path)?, portion, at),
    }
}

/// `pts` when every coordinate is finite.
fn finite(pts: Vec<Point>) -> Option<Vec<Point>> {
    pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(pts)
}

/// The runs of the right-angled route `pts` with run `portion` moved to `p`.
fn reshaped_runs(pts: &[Point], portion: usize, p: Point) -> Option<ConnectorPath> {
    let last = pts.len().checked_sub(2)?;
    let (first, end) = (*pts.first()?, *pts.last()?);
    if portion > last || last + 3 > MAX_RUNS {
        return None;
    }
    let mid = first.midpoint(end);
    let across = |i: usize| pts[i].y == pts[i + 1].y;
    // Run i's place (where it lies), and a point's place along it, from the middle of the ends.
    let lies = |i: usize, q: Point| if across(i) { q.y - mid.y } else { q.x - mid.x };
    let along = |i: usize, q: Point| if across(i) { q.x - mid.x } else { q.y - mid.y };
    // The stretch of run i kept on its end `q`, towards the run's other end `to`.
    let stub = |i: usize, q: Point, to: Point| {
        let d = to - q;
        along(i, q + d * (ROOM.min(d.hypot() / 2.0) / d.hypot().max(TINY)))
    };
    let mut at: Vec<f64> = (1..last).map(|i| lies(i, pts[i])).collect();
    let moved = lies(portion, p);
    if last == 0 {
        at = vec![stub(0, first, end), moved, stub(0, end, first)];
    } else if portion == 0 {
        at.splice(0..0, [stub(0, first, pts[1]), moved]);
    } else if portion == last {
        at.extend([moved, stub(last, end, pts[last])]);
    } else {
        *at.get_mut(portion - 1)? = moved;
    }
    Some(ConnectorPath::Runs { across: across(0), at })
}

/// The right-angled route from `p0` to `p1` through runs at the places `at` (see
/// [`ConnectorPath::Runs`]).
fn along_runs(p0: Point, p1: Point, across: bool, at: &[f64]) -> Vec<Point> {
    let mid = p0.midpoint(p1);
    let (mut out, mut cur, mut sideways) = (vec![p0], p0, across);
    for v in at {
        cur = if sideways { Point::new(mid.x + v, cur.y) } else { Point::new(cur.x, mid.y + v) };
        out.push(cur);
        sideways = !sideways;
    }
    out.push(if sideways { Point::new(p1.x, cur.y) } else { Point::new(cur.x, p1.y) });
    out.push(p1);
    tidy(out)
}

/// The middle of the curve whose ends and control points are `pts`.
fn curve_middle(pts: &[Point]) -> Option<Point> {
    let [a, b, c, d] = pts else { return None };
    Some(((a.to_vec2() + (b.to_vec2() + c.to_vec2()) * 3.0 + d.to_vec2()) / 8.0).to_point())
}

/// `pts` without repeats or turns that don't turn (three in a line across or down).
fn tidy(pts: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last() == Some(&p) {
            continue;
        }
        if let [.., a, b] = out.as_slice()
            && ((a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y))
        {
            out.pop();
        }
        out.push(p);
    }
    out
}

/// Where end `e` attaches (its point, or the middle of its box's side facing `other`) and that
/// side (`None`: a free point).
fn attach(e: RouteEnd, other: Point) -> (Point, Option<ConnectionSide>) {
    match e {
        RouteEnd::Point(p) => (p, None),
        RouteEnd::Box(b, side) => {
            let side = named_or_facing(b, side, other);
            (side.point(b), Some(side))
        }
        RouteEnd::On(b, side, p) => (p, Some(named_or_facing(b, side, other))),
    }
}

/// `side`, or for [`ConnectionSide::Auto`] the side of `b` facing `other`.
fn named_or_facing(b: Rect, side: ConnectionSide, other: Point) -> ConnectionSide {
    if side != ConnectionSide::Auto {
        return side;
    }
    let d = other - b.center();
    // Compare against the box's proportions: a wide box takes the sides more.
    if d.x.abs() * b.height().max(TINY) >= d.y.abs() * b.width().max(TINY) {
        if d.x >= 0.0 { ConnectionSide::Right } else { ConnectionSide::Left }
    } else if d.y >= 0.0 {
        ConnectionSide::Bottom
    } else {
        ConnectionSide::Top
    }
}

/// A curve from `p0` to `p1` leaving and arriving along the sides' normals (a free end pulls no
/// way): the ends and the two control points between them.
fn curve(p0: Point, s0: Option<ConnectionSide>, p1: Point, s1: Option<ConnectionSide>) -> Vec<Point> {
    let reach = (p1 - p0).hypot() / 2.0;
    let pull = |p: Point, s: Option<ConnectionSide>| p + s.map_or(Vec2::ZERO, |s| s.normal() * reach);
    vec![p0, pull(p0, s0), pull(p1, s1), p1]
}

/// A right-angled route from `p0` to `p1`, leaving and arriving along the sides' normals (`s`; a
/// free end follows the other): the short way when it keeps out of the attached objects' `boxes`,
/// else the cheapest of a few ways around them (by length, and turns).
fn elbow(p0: Point, s0: Option<ConnectionSide>, p1: Point, s1: Option<ConnectionSide>, boxes: &[Rect]) -> Vec<Point> {
    let direct = tidy(short_elbow(p0, s0.map(ConnectionSide::horizontal), p1, s1.map(ConnectionSide::horizontal)));
    let Some(all) = boxes.iter().copied().reduce(|a, b| a.union(b)) else { return direct };
    if !crosses(&direct, boxes) {
        return direct;
    }
    // Out of each end by ROOM, then across on one of a few lines: halfway, or clear of the objects.
    let out = |p: Point, s: Option<ConnectionSide>| p + s.map_or(Vec2::ZERO, |s| s.normal() * ROOM);
    let (a, b) = (out(p0, s0), out(p1, s1));
    let mut ways = vec![vec![p0, a, Point::new(b.x, a.y), b, p1], vec![p0, a, Point::new(a.x, b.y), b, p1]];
    for x in [(a.x + b.x) / 2.0, all.x0 - ROOM, all.x1 + ROOM] {
        ways.push(vec![p0, a, Point::new(x, a.y), Point::new(x, b.y), b, p1]);
    }
    for y in [(a.y + b.y) / 2.0, all.y0 - ROOM, all.y1 + ROOM] {
        ways.push(vec![p0, a, Point::new(a.x, y), Point::new(b.x, y), b, p1]);
    }
    let cost = |r: &Vec<Point>| r.windows(2).map(|s| (s[1] - s[0]).hypot()).sum::<f64>() + ROOM * r.len() as f64;
    ways.into_iter().map(tidy).filter(|r| !crosses(r, boxes)).min_by(|x, y| cost(x).total_cmp(&cost(y))).unwrap_or(direct)
}

/// Whether the right-angled route `pts` runs through the inside of any of `boxes` (along an edge,
/// or out of one, is outside).
fn crosses(pts: &[Point], boxes: &[Rect]) -> bool {
    pts.windows(2).any(|s| {
        let (a, b) = (s[0], s[1]);
        boxes.iter().any(|r| a.x.min(b.x) < r.x1 - EDGE && a.x.max(b.x) > r.x0 + EDGE && a.y.min(b.y) < r.y1 - EDGE && a.y.max(b.y) > r.y0 + EDGE)
    })
}

/// The short right-angled route from `p0` to `p1` (`h`: the side faces sideways; a free end
/// follows the other): halfway across between two sides facing the same way, one turn otherwise.
fn short_elbow(p0: Point, h0: Option<bool>, p1: Point, h1: Option<bool>) -> Vec<Point> {
    let h0 = h0.or(h1).unwrap_or((p1.x - p0.x).abs() >= (p1.y - p0.y).abs());
    let h1 = h1.unwrap_or(h0);
    match (h0, h1) {
        (true, true) => {
            let mx = (p0.x + p1.x) / 2.0;
            vec![p0, Point::new(mx, p0.y), Point::new(mx, p1.y), p1]
        }
        (false, false) => {
            let my = (p0.y + p1.y) / 2.0;
            vec![p0, Point::new(p0.x, my), Point::new(p1.x, my), p1]
        }
        (true, false) => vec![p0, Point::new(p1.x, p0.y), p1],
        (false, true) => vec![p0, Point::new(p0.x, p1.y), p1],
    }
}
