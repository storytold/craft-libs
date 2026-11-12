//! Geometry the caller owns: how a connector is routed between its ends, and whether a text path
//! can be measured by the caller's text layout. The mapping reads where a connector's ends are and
//! how the file says it runs; the route between them is the caller's (an editor routes its own
//! connectors the same way, so an imported one looks like one drawn there).

use kurbo::{Point, Rect, Vec2};

/// Most runs of a saved elbow path ([`SavedRuns`]) the mapping reads.
pub const MAX_RUNS: usize = 64;

/// How a connector runs between its ends (`connectorLineStyle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Routing {
    #[default]
    Straight,
    /// Right angles only (`ELBOWED`).
    Elbow,
    /// One smooth curve (`CURVED`).
    Curved,
}

/// The side of a layer a connector end attaches to (at its middle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    /// The side facing the other end.
    #[default]
    Auto,
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    /// The outward normal of the side (zero for [`Side::Auto`]).
    pub fn normal(self) -> Vec2 {
        match self {
            Side::Auto => Vec2::ZERO,
            Side::Top => Vec2::new(0.0, -1.0),
            Side::Right => Vec2::new(1.0, 0.0),
            Side::Bottom => Vec2::new(0.0, 1.0),
            Side::Left => Vec2::new(-1.0, 0.0),
        }
    }
}

/// A connector end, in the coordinates of the connector's parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteEnd {
    /// A free point, or a point on a layer the line goes straight to.
    Point(Point),
    /// A layer's bounding box and the side the end leaves from.
    Box(Rect, Side),
}

/// The path a FigJam elbow connector was saved with: the runs between the first and the last,
/// alternating direction. `across`: the first saved run goes across (horizontally); `at`: each
/// saved run's offset from the middle of the two attach points (y for a run across, x for one
/// down).
#[derive(Clone, Debug, PartialEq)]
pub struct SavedRuns {
    pub across: bool,
    pub at: Vec<f64>,
}

/// The caller's geometry.
pub trait Geometry {
    /// The points of the line from `from` to `to`: a polyline, or for [`Routing::Curved`] the
    /// start, two control points and the end. `saved`: an elbow's saved path, when it has one
    /// that fits. `None` when the ends can't be joined.
    fn route(&self, from: RouteEnd, to: RouteEnd, routing: Routing, saved: Option<&SavedRuns>) -> Option<Vec<Point>>;

    /// Where the line leaves `from` and arrives at `to`, and the sides it attaches to (`None` for
    /// a free point).
    fn attach(&self, from: RouteEnd, to: RouteEnd) -> ((Point, Option<Side>), (Point, Option<Side>));

    /// Whether the caller's text layout can measure text along the SVG path `d`. A text path it
    /// can't is left out with a warning.
    fn text_path_measurable(&self, _d: &str) -> bool {
        true
    }
}

/// Geometry without routing of its own: straight lines between the ends (a box's end at the
/// middle of its side, or at its centre for [`Side::Auto`]), elbows and curves included, and
/// every text path accepted.
#[derive(Clone, Copy, Debug, Default)]
pub struct Straight;

impl Geometry for Straight {
    fn route(&self, from: RouteEnd, to: RouteEnd, _: Routing, _: Option<&SavedRuns>) -> Option<Vec<Point>> {
        let ((a, _), (b, _)) = self.attach(from, to);
        Some(vec![a, b])
    }

    fn attach(&self, from: RouteEnd, to: RouteEnd) -> ((Point, Option<Side>), (Point, Option<Side>)) {
        let at = |e: RouteEnd| match e {
            RouteEnd::Point(p) => (p, None),
            RouteEnd::Box(b, Side::Auto) => (b.center(), Some(Side::Auto)),
            RouteEnd::Box(b, s) => {
                let n = s.normal();
                (b.center() + Vec2::new(n.x * b.width() / 2.0, n.y * b.height() / 2.0), Some(s))
            }
        };
        (at(from), at(to))
    }
}
