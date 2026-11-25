//! The Parametric Shape Tool's geometry (Path Precision Tools): shapes described by a few numbers
//! (kind, centre, size, rotation, a count and up to two values of the kind's own, corner radius)
//! that stay editable as those numbers. [`ParametricShape::path`] draws one; [`fingerprint`] and
//! [`ParametricShape::recover`] tell whether a path is still the shape it was drawn as, moved,
//! turned or scaled since, or was edited into something else; [`recognise`] finds the shape a
//! plain rectangle, ellipse or regular polygon is.

use std::f64::consts::{PI, TAU};

use craft_geom::corners::cut_corners;
use craft_geom::shapes::KAPPA;
use craft_geom::{Anchor, AnchorKind, PathData, SubPath};
use kurbo::{Affine, Point, Vec2};

/// The fewest sides, points or teeth.
pub const MIN_COUNT: u32 = 3;
/// The most sides, points or teeth.
pub const MAX_COUNT: u32 = 1000;
/// The smallest width or height (pt).
pub const MIN_SIZE: f64 = 0.01;
/// The largest width, height or corner radius (pt).
pub const MAX_SIZE: f64 = 16384.0;
/// The farthest from the origin a centre may be (pt).
pub const MAX_CENTER: f64 = 1e7;

/// The kinds of parametric shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Polygon,
    Star,
    Ring,
    Arrow,
    Cross,
    Heart,
    Gear,
    Bubble,
}

/// One of a kind's own values: its range and default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValueRange {
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

const fn range(min: f64, max: f64, default: f64) -> Option<ValueRange> {
    Some(ValueRange { min, max, default })
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 10] = [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Polygon,
        ShapeKind::Star,
        ShapeKind::Ring,
        ShapeKind::Arrow,
        ShapeKind::Cross,
        ShapeKind::Heart,
        ShapeKind::Gear,
        ShapeKind::Bubble,
    ];

    /// The kind's name in commands and files.
    pub fn id(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "rectangle",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Polygon => "polygon",
            ShapeKind::Star => "star",
            ShapeKind::Ring => "ring",
            ShapeKind::Arrow => "arrow",
            ShapeKind::Cross => "cross",
            ShapeKind::Heart => "heart",
            ShapeKind::Gear => "gear",
            ShapeKind::Bubble => "bubble",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Does `count` set how many sides, points or teeth the kind has?
    pub fn counted(self) -> bool {
        matches!(self, ShapeKind::Polygon | ShapeKind::Star | ShapeKind::Gear)
    }

    /// The count a shape of this kind starts with.
    pub fn default_count(self) -> u32 {
        match self {
            ShapeKind::Star => 5,
            ShapeKind::Gear => 12,
            _ => 6,
        }
    }

    /// Can its corners be rounded (`radius`)? Not a shape without corners.
    pub fn rounds(self) -> bool {
        !matches!(self, ShapeKind::Ellipse | ShapeKind::Ring)
    }

    /// The kind's own values `a` and `b` (as fractions of its size), where it has them: a star's
    /// and a ring's inner radius; an arrow's head length and shaft width; a cross's bar offset
    /// (−: up) and arm thickness; a heart's notch depth and the roundness of its point; a gear's
    /// tooth height; a speech bubble's tail height and where its tip is across (−1 left, 1 right).
    pub fn values(self) -> [Option<ValueRange>; 2] {
        match self {
            ShapeKind::Star => [range(0.01, 1.0, 0.5), None],
            ShapeKind::Ring => [range(0.01, 0.99, 0.5), None],
            ShapeKind::Arrow => [range(0.05, 0.95, 0.4), range(0.05, 0.95, 0.4)],
            ShapeKind::Cross => [range(-0.9, 0.9, 0.0), range(0.05, 0.95, 1.0 / 3.0)],
            ShapeKind::Heart => [range(0.0, 1.0, 0.4), range(0.0, 1.0, 0.0)],
            ShapeKind::Gear => [range(0.01, 0.9, 0.2), None],
            ShapeKind::Bubble => [range(0.05, 0.9, 0.25), range(-1.0, 1.0, -0.4)],
            ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Polygon => [None, None],
        }
    }
}

/// A parametric shape: what it is and where; its outline is [`ParametricShape::path`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParametricShape {
    pub kind: ShapeKind,
    pub center: Point,
    pub width: f64,
    pub height: f64,
    /// Degrees clockwise on the page; 0 stands the shape upright (a polygon's first corner up).
    pub rotation: f64,
    /// Sides, points or teeth ([`ShapeKind::counted`]).
    pub count: u32,
    /// The kind's own values ([`ShapeKind::values`]); a kind without one ignores it.
    pub a: f64,
    pub b: f64,
    /// The corners' radius (pt), each no larger than fits ([`ShapeKind::rounds`]).
    pub radius: f64,
}

/// Where a shape is wrong, as `(param, min, max)`: `param` must be from `min` to `max`.
pub type OutOfRange = (&'static str, f64, f64);

impl ParametricShape {
    /// A `kind` of shape about `center`, `width` × `height`, with the kind's defaults.
    pub fn new(kind: ShapeKind, center: Point, width: f64, height: f64) -> Self {
        let [a, b] = kind.values().map(|v| v.map_or(0.0, |v| v.default));
        Self { kind, center, width, height, rotation: 0.0, count: kind.default_count(), a, b, radius: 0.0 }
    }

    /// The shape as another kind, keeping its place, size, rotation and corner radius; its count
    /// and values are the new kind's defaults.
    pub fn with_kind(self, kind: ShapeKind) -> Self {
        if kind == self.kind {
            return self;
        }
        Self { radius: self.radius, rotation: self.rotation, ..Self::new(kind, self.center, self.width, self.height) }
    }

    /// The first number out of its range, if any (every number must also be finite).
    pub fn check(&self) -> Result<(), OutOfRange> {
        let within = |v: f64, lo: f64, hi: f64| v.is_finite() && (lo..=hi).contains(&v);
        let size = [("width", self.width), ("height", self.height)];
        if let Some((key, _)) = size.into_iter().find(|(_, v)| !within(*v, MIN_SIZE, MAX_SIZE)) {
            return Err((key, MIN_SIZE, MAX_SIZE));
        }
        if !within(self.radius, 0.0, MAX_SIZE) {
            return Err(("radius", 0.0, MAX_SIZE));
        }
        if !(within(self.center.x, -MAX_CENTER, MAX_CENTER) && within(self.center.y, -MAX_CENTER, MAX_CENTER)) {
            return Err(("center", -MAX_CENTER, MAX_CENTER));
        }
        if !self.rotation.is_finite() {
            return Err(("rotation", -360.0, 360.0));
        }
        if !(MIN_COUNT..=MAX_COUNT).contains(&self.count) {
            return Err(("count", f64::from(MIN_COUNT), f64::from(MAX_COUNT)));
        }
        // A value the kind doesn't have is left alone.
        for (key, v, spec) in [("a", self.a, self.kind.values()[0]), ("b", self.b, self.kind.values()[1])] {
            if let Some(r) = spec.filter(|r| !within(v, r.min, r.max)) {
                return Err((key, r.min, r.max));
            }
        }
        Ok(())
    }

    /// From the shape's own frame (its centre at the origin, upright) to the page.
    pub fn frame(&self) -> Affine {
        Affine::translate(self.center.to_vec2()) * Affine::rotate(self.rotation.to_radians())
    }

    /// The shape's outline on the page: closed subpaths, starting at the top (a rectangle's and a
    /// speech bubble's at the top left corner) and running clockwise; a ring's hole runs the
    /// other way. `None` when a number is out of range ([`Self::check`]).
    pub fn path(&self) -> Option<PathData> {
        self.check().ok()?;
        let (rx, ry) = (self.width / 2.0, self.height / 2.0);
        let at = |x: f64, y: f64| Point::new(x * rx, y * ry);
        let polygon = |pts: Vec<Point>| vec![SubPath::polyline(&pts, true)];
        let (a, b) = (self.a, self.b);
        let around = |n: usize, turn: f64, radius_at: &dyn Fn(usize) -> f64| -> Vec<Point> {
            (0..n)
                .map(|i| {
                    let (s, c) = (i as f64 * turn).sin_cos();
                    let r = radius_at(i);
                    at(r * s, -r * c)
                })
                .collect()
        };
        let n = self.count as usize;
        let subpaths = match self.kind {
            ShapeKind::Rectangle => polygon(vec![at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]),
            ShapeKind::Ellipse => vec![ellipse(rx, ry)],
            ShapeKind::Polygon => polygon(around(n, TAU / n as f64, &|_| 1.0)),
            ShapeKind::Star => polygon(around(2 * n, PI / n as f64, &|i| if i % 2 == 0 { 1.0 } else { a })),
            ShapeKind::Ring => {
                let mut hole = ellipse(rx * a, ry * a);
                hole.reverse();
                vec![ellipse(rx, ry), hole]
            }
            ShapeKind::Arrow => {
                // The head from the top down to `yb`, the shaft below it.
                let (yb, sw) = (-1.0 + 2.0 * a, b);
                polygon(vec![at(0.0, -1.0), at(1.0, yb), at(sw, yb), at(sw, 1.0), at(-sw, 1.0), at(-sw, yb), at(-1.0, yb)])
            }
            ShapeKind::Cross => {
                // The bar across, `a` of the way from the middle to an end of the upright.
                let (t, y0) = (b, a * (1.0 - b));
                let (top, bottom) = (y0 - t, y0 + t);
                polygon(vec![
                    at(-t, -1.0),
                    at(t, -1.0),
                    at(t, top),
                    at(1.0, top),
                    at(1.0, bottom),
                    at(t, bottom),
                    at(t, 1.0),
                    at(-t, 1.0),
                    at(-t, bottom),
                    at(-1.0, bottom),
                    at(-1.0, top),
                    at(-t, top),
                ])
            }
            ShapeKind::Heart => vec![heart(a, b, &at)],
            ShapeKind::Gear => {
                // Each tooth's flanks and tip, centred on its angle; straight roots between them.
                let step = TAU / n as f64;
                let root = 1.0 - a;
                let mut pts = Vec::with_capacity(4 * n);
                for i in 0..n {
                    for (f, r) in [(-0.3, root), (-0.15, 1.0), (0.15, 1.0), (0.3, root)] {
                        let (s, c) = ((i as f64 + f) * step).sin_cos();
                        pts.push(at(r * s, -r * c));
                    }
                }
                polygon(pts)
            }
            ShapeKind::Bubble => {
                // The body above `by`, the tail's base on its bottom edge, its tip on the bottom.
                let by = 1.0 - 2.0 * a;
                let (mid, half) = (b * 0.5, 0.3);
                polygon(vec![at(-1.0, -1.0), at(1.0, -1.0), at(1.0, by), at(mid + half, by), at(b, 1.0), at(mid - half, by), at(-1.0, by)])
            }
        };
        let mut path = PathData::new(subpaths);
        if self.radius > 0.0 && self.kind.rounds() {
            path = cut_corners(&path, &vec![self.radius; path.anchor_count()], &[]).0;
        }
        path.transform(self.frame());
        Some(path)
    }

    /// The shape `path` is when it is this shape's outline moved, turned or scaled (along the
    /// shape's own axes; uniformly when its corners are rounded), not mirrored or skewed, and
    /// otherwise unedited: the new centre, size, rotation and corner radius. `None` when its
    /// anchors were edited, or it was skewed or mirrored.
    pub fn recover(&self, path: &PathData) -> Option<Self> {
        let drawn = self.path()?;
        let pts = |p: &PathData| p.anchors().map(|(_, _, a)| a.p).collect::<Vec<_>>();
        let (from, to) = (pts(&drawn), pts(path));
        if from.len() != to.len() || drawn.subpaths.len() != path.subpaths.len() {
            return None;
        }
        let m = fit_affine(&from, &to)? * self.frame();
        let [m0, m1, m2, m3, ..] = m.as_coeffs();
        let (x, y) = (Vec2::new(m0, m1), Vec2::new(m2, m3));
        let (sx, sy) = (x.hypot(), y.hypot());
        // Square and not mirrored: the shape's axes stay at right angles, turned the same way.
        if !(sx > 1e-12 && sy > 1e-12) || x.dot(y).abs() > 1e-6 * sx * sy || x.cross(y) <= 0.0 {
            return None;
        }
        let out = Self {
            center: m * Point::ZERO,
            width: self.width * sx,
            height: self.height * sy,
            rotation: craft_geom::normalize_deg(x.y.atan2(x.x).to_degrees()),
            radius: self.radius * (sx * sy).sqrt(),
            ..*self
        };
        out.check().ok()?;
        same(&out.path()?, path).then_some(out)
    }
}

/// An ellipse of radii `rx`, `ry` about the origin: four smooth anchors from the top, clockwise.
fn ellipse(rx: f64, ry: f64) -> SubPath {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let s = |p: Point, out: Vec2| Anchor { p, h_in: p - out, h_out: p + out, kind: AnchorKind::Smooth };
    SubPath::new(
        vec![
            s(Point::new(0.0, -ry), Vec2::new(kx, 0.0)),
            s(Point::new(rx, 0.0), Vec2::new(0.0, ky)),
            s(Point::new(0.0, ry), Vec2::new(-kx, 0.0)),
            s(Point::new(-rx, 0.0), Vec2::new(0.0, -ky)),
        ],
        true,
    )
}

/// A heart in the unit box (`at` scales it): two lobes over a point at the bottom, the notch
/// between the lobes `depth` of the way down to the middle, the point rounder with `round`.
fn heart(depth: f64, round: f64, at: &dyn Fn(f64, f64) -> Point) -> SubPath {
    let ny = -1.0 + depth;
    let notch = at(0.0, ny);
    let mirror = |x: f64, y: f64| [at(x, y), at(-x, y)];
    let [notch_out, notch_in] = mirror(0.1, ny - 0.3 * (1.0 + ny));
    let [tip_in, tip_out] = mirror(0.2 + 0.4 * round, 0.7 + 0.3 * round);
    let a = |p, h_in, h_out, kind| Anchor { p, h_in, h_out, kind };
    let smooth = AnchorKind::Smooth;
    SubPath::new(
        vec![
            a(notch, notch_in, notch_out, AnchorKind::Corner),
            a(at(0.5, -1.0), at(0.2, -1.0), at(0.8, -1.0), smooth),
            a(at(1.0, -0.35), at(1.0, -0.75), at(1.0, 0.1), smooth),
            a(at(0.0, 1.0), tip_in, tip_out, AnchorKind::Corner),
            a(at(-1.0, -0.35), at(-1.0, 0.1), at(-1.0, -0.75), smooth),
            a(at(-0.5, -1.0), at(-0.8, -1.0), at(-0.2, -1.0), smooth),
        ],
        true,
    )
}

/// The affine map taking each of `from` to the point of `to` at its index, worked out from
/// three of them far apart; `None` when they all lie on a line.
fn fit_affine(from: &[Point], to: &[Point]) -> Option<Affine> {
    let (&p0, &q0) = (from.first()?, to.first()?);
    let far = (0..from.len()).max_by(|&i, &j| dist(from, i, p0).total_cmp(&dist(from, j, p0)))?;
    let p1 = *from.get(far)?;
    let wide = |i: usize| from.get(i).map_or(0.0, |p| (p1 - p0).cross(*p - p0).abs());
    let third = (0..from.len()).max_by(|&i, &j| wide(i).total_cmp(&wide(j)))?;
    let (p2, q1, q2) = (*from.get(third)?, *to.get(far)?, *to.get(third)?);
    let basis = |o: Point, u: Vec2, v: Vec2| Affine::new([u.x, u.y, v.x, v.y, o.x, o.y]);
    let g = basis(p0, p1 - p0, p2 - p0);
    let det = g.determinant();
    // Too thin a triangle to tell a turn from a skew.
    if det.is_nan() || det.abs() <= 1e-9 * (p1 - p0).hypot2().max(1e-12) {
        return None;
    }
    Some(basis(q0, q1 - q0, q2 - q0) * g.inverse())
}

fn dist(pts: &[Point], i: usize, from: Point) -> f64 {
    pts.get(i).map_or(0.0, |p| p.distance(from))
}

/// Are `a` and `b` the same outline, anchor for anchor and handle for handle (to well under a
/// thousandth of a point at the size of the art)?
fn same(a: &PathData, b: &PathData) -> bool {
    let scale = a.anchors().map(|(_, _, x)| x.p.x.abs().max(x.p.y.abs())).fold(1.0, f64::max);
    let tol = 1e-3_f64.max(scale * 1e-9);
    a.subpaths.len() == b.subpaths.len()
        && a.subpaths.iter().zip(&b.subpaths).all(|(s, t)| {
            s.closed == t.closed
                && s.anchors.len() == t.anchors.len()
                && s.anchors
                    .iter()
                    .zip(&t.anchors)
                    .all(|(x, y)| x.p.distance(y.p) <= tol && x.h_in.distance(y.h_in) <= tol && x.h_out.distance(y.h_out) <= tol)
        })
}

/// A fingerprint of `path`'s geometry: its subpaths and every anchor and handle to a thousandth of
/// a point. Kept with a shape, it shows whether the path is still the one drawn.
pub fn fingerprint(path: &PathData) -> u64 {
    // FNV-1a: the same in every build, as a fingerprint saved in a file must be.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: i64| {
        for byte in v.to_le_bytes() {
            h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for sp in &path.subpaths {
        eat(sp.anchors.len() as i64);
        eat(i64::from(sp.closed));
        for a in &sp.anchors {
            for p in [a.p, a.h_in, a.h_out] {
                // Saturating casts: a huge or non-finite coordinate still hashes.
                eat((p.x * 1000.0).round() as i64);
                eat((p.y * 1000.0).round() as i64);
            }
        }
    }
    h
}

/// The parametric shape a plain closed path is, if it is one: a rectangle (four corners, no
/// handles), an ellipse (four smooth anchors) or a polygon whose sides and corners are all equal,
/// stretched along its own axes at most. The shape's outline starts at its own first anchor, so
/// the path's anchors may come in a different order.
pub fn recognise(path: &PathData) -> Option<ParametricShape> {
    let [sp] = path.subpaths.as_slice() else { return None };
    let n = sp.anchors.len();
    if !sp.closed || !(3..=MAX_COUNT as usize).contains(&n) {
        return None;
    }
    let corners = sp.anchors.iter().all(|a| !a.has_in() && !a.has_out());
    let kinds: &[ShapeKind] = match (n, corners) {
        (4, true) => &[ShapeKind::Rectangle, ShapeKind::Polygon],
        (4, false) => &[ShapeKind::Ellipse],
        (_, true) => &[ShapeKind::Polygon],
        _ => return None,
    };
    let unit = |kind| ParametricShape { count: n as u32, ..ParametricShape::new(kind, Point::ZERO, 2.0, 2.0) };
    // The path read from each anchor in turn, either way round.
    for reversed in [false, true] {
        let mut sp = sp.clone();
        if reversed {
            sp.reverse();
        }
        for _ in 0..n {
            let candidate = PathData::single(sp.clone());
            if let Some(s) = kinds.iter().find_map(|k| unit(*k).recover(&candidate)) {
                return Some(s);
            }
            sp.anchors.rotate_left(1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(kind: ShapeKind) -> ParametricShape {
        ParametricShape::new(kind, Point::new(100.0, 50.0), 80.0, 60.0)
    }

    fn closed(p: &PathData) -> bool {
        p.subpaths.iter().all(|s| s.closed)
    }

    /// Every point of `p` lies in the shape's box (a little slack for curves).
    fn within_box(p: &PathData, s: &ParametricShape) -> bool {
        let b = p.bounds().unwrap();
        let (hw, hh) = (s.width / 2.0 + 1e-6, s.height / 2.0 + 1e-6);
        b.x0 >= s.center.x - hw && b.x1 <= s.center.x + hw && b.y0 >= s.center.y - hh && b.y1 <= s.center.y + hh
    }

    #[test]
    fn each_kind_draws_a_closed_outline_in_its_box() {
        for kind in ShapeKind::ALL {
            let s = shape(kind);
            assert_eq!(ShapeKind::parse(kind.id()), Some(kind));
            assert_eq!(s.check(), Ok(()), "{kind:?}");
            let p = s.path().unwrap();
            assert!(closed(&p) && within_box(&p, &s), "{kind:?}: {:?}", p.bounds());
            assert!(p.anchors().all(|(_, _, a)| a.p.x.is_finite() && a.p.y.is_finite()));
        }
        assert_eq!(ShapeKind::parse("cloud"), None);
    }

    #[test]
    fn anchor_counts_and_corners_per_kind() {
        let count = |kind, n: u32| ParametricShape { count: n, ..shape(kind) }.path().unwrap().anchor_count();
        assert_eq!(count(ShapeKind::Rectangle, 3), 4);
        assert_eq!(count(ShapeKind::Ellipse, 3), 4);
        assert_eq!(count(ShapeKind::Polygon, 7), 7);
        assert_eq!(count(ShapeKind::Polygon, 1000), 1000);
        assert_eq!(count(ShapeKind::Star, 5), 10);
        assert_eq!(count(ShapeKind::Gear, 12), 48);
        assert_eq!(count(ShapeKind::Arrow, 3), 7);
        assert_eq!(count(ShapeKind::Cross, 3), 12);
        assert_eq!(count(ShapeKind::Heart, 3), 6);
        assert_eq!(count(ShapeKind::Bubble, 3), 7);
        let ring = shape(ShapeKind::Ring).path().unwrap();
        assert_eq!(ring.subpaths.len(), 2);
        // The hole runs the other way, so it stays a hole under the non-zero rule.
        assert!(ring.subpaths[0].signed_area() * ring.subpaths[1].signed_area() < 0.0);
        let (outer, inner) = (ring.subpaths[0].area().abs(), ring.subpaths[1].area().abs());
        assert!((inner / outer - 0.25).abs() < 1e-3, "inner ratio 0.5 → a quarter of the area: {}", inner / outer);
    }

    #[test]
    fn polygons_and_stars_stand_upright_and_turn_clockwise() {
        let p = shape(ShapeKind::Polygon).path().unwrap();
        let top = p.subpaths[0].anchors[0].p;
        assert!(top.distance(Point::new(100.0, 20.0)) < 1e-9, "first corner straight up: {top:?}");
        let turned = ParametricShape { rotation: 90.0, ..shape(ShapeKind::Polygon) }.path().unwrap();
        let right = turned.subpaths[0].anchors[0].p;
        assert!(right.distance(Point::new(130.0, 50.0)) < 1e-9, "a quarter turn clockwise puts it on the right: {right:?}");
        let star = shape(ShapeKind::Star).path().unwrap();
        let inner = star.subpaths[0].anchors[1].p.distance(Point::new(100.0, 50.0));
        let outer = star.subpaths[0].anchors[0].p.distance(Point::new(100.0, 50.0));
        assert!((outer - 30.0).abs() < 1e-9 && inner < outer * 0.6, "{outer} {inner}");
    }

    #[test]
    fn values_change_each_kind() {
        let differ = |kind: ShapeKind, a: f64, b: f64| {
            let s = shape(kind);
            let t = ParametricShape { a, b, ..s };
            assert_eq!(t.check(), Ok(()), "{kind:?}");
            assert_ne!(s.path(), t.path(), "{kind:?} a {a} b {b}");
        };
        differ(ShapeKind::Star, 0.2, 0.0);
        differ(ShapeKind::Ring, 0.8, 0.0);
        differ(ShapeKind::Arrow, 0.6, 0.4);
        differ(ShapeKind::Arrow, 0.4, 0.2);
        differ(ShapeKind::Cross, 0.5, 1.0 / 3.0);
        differ(ShapeKind::Cross, 0.0, 0.6);
        differ(ShapeKind::Heart, 0.8, 0.0);
        differ(ShapeKind::Heart, 0.4, 1.0);
        differ(ShapeKind::Gear, 0.5, 0.0);
        differ(ShapeKind::Bubble, 0.5, -0.4);
        differ(ShapeKind::Bubble, 0.25, 0.8);
        // An arrow's head reaches down `a` of the height.
        let arrow = ParametricShape { a: 0.5, ..shape(ShapeKind::Arrow) }.path().unwrap();
        assert!((arrow.subpaths[0].anchors[1].p.y - 50.0).abs() < 1e-9);
        // A gear's roots sit `a` of the radius in.
        let gear = shape(ShapeKind::Gear).path().unwrap();
        let root = gear.subpaths[0].anchors[0].p - Point::new(100.0, 50.0);
        assert!((root.x / 40.0).hypot(root.y / 30.0) - 0.8 < 1e-9);
    }

    #[test]
    fn corners_round_but_never_past_the_middle_of_a_side() {
        let sharp = shape(ShapeKind::Rectangle);
        let round = ParametricShape { radius: 10.0, ..sharp };
        let p = round.path().unwrap();
        assert_eq!(p.anchor_count(), 8);
        assert_eq!(p.bounds(), sharp.path().unwrap().bounds());
        // Far too big a radius: the corners meet in the middle of the short sides.
        let huge = ParametricShape { radius: 1000.0, ..sharp }.path().unwrap();
        let b = huge.bounds().unwrap();
        assert!((b.width() - 80.0).abs() < 1e-6 && (b.height() - 60.0).abs() < 1e-6);
        // Kinds without corners ignore it.
        let e = ParametricShape { radius: 10.0, ..shape(ShapeKind::Ellipse) };
        assert_eq!(e.path(), shape(ShapeKind::Ellipse).path());
        for kind in ShapeKind::ALL.into_iter().filter(|k| k.rounds()) {
            let p = ParametricShape { radius: 3.0, ..shape(kind) }.path().unwrap();
            assert!(p.anchor_count() > shape(kind).path().unwrap().anchor_count(), "{kind:?}");
        }
    }

    #[test]
    fn numbers_out_of_range_are_refused() {
        let s = shape(ShapeKind::Star);
        let bad = [
            (ParametricShape { width: 0.0, ..s }, "width"),
            (ParametricShape { height: MAX_SIZE * 2.0, ..s }, "height"),
            (ParametricShape { width: f64::NAN, ..s }, "width"),
            (ParametricShape { count: 2, ..s }, "count"),
            (ParametricShape { count: MAX_COUNT + 1, ..s }, "count"),
            (ParametricShape { a: 1.5, ..s }, "a"),
            (ParametricShape { a: f64::NAN, ..s }, "a"),
            (ParametricShape { radius: -1.0, ..s }, "radius"),
            (ParametricShape { rotation: f64::INFINITY, ..s }, "rotation"),
            (ParametricShape { center: Point::new(1e300, 0.0), ..s }, "center"),
        ];
        for (shape, key) in bad {
            assert_eq!(shape.check().map_err(|e| e.0), Err(key));
            assert_eq!(shape.path(), None);
        }
    }

    #[test]
    fn with_kind_keeps_the_place_and_takes_the_new_kinds_defaults() {
        let s = ParametricShape { rotation: 30.0, radius: 4.0, count: 9, ..shape(ShapeKind::Polygon) };
        let star = s.with_kind(ShapeKind::Star);
        assert_eq!((star.center, star.width, star.rotation, star.radius, star.count, star.a), (s.center, 80.0, 30.0, 4.0, 5, 0.5));
        let arrow = star.with_kind(ShapeKind::Arrow);
        assert_eq!((arrow.count, arrow.a, arrow.b), (6, 0.4, 0.4));
        assert_eq!(arrow.check(), Ok(()));
    }

    #[test]
    fn a_moved_turned_or_scaled_shape_is_recovered() {
        for kind in ShapeKind::ALL {
            let s = ParametricShape { rotation: 20.0, ..shape(kind) };
            let p = s.path().unwrap();
            assert_eq!(s.recover(&p).map(|r| r.kind), Some(kind), "unchanged {kind:?}");
            let xf = Affine::translate((40.0, -15.0)) * Affine::rotate_about(0.5, s.center);
            let r = s.recover(&p.transformed(xf)).unwrap_or_else(|| panic!("{kind:?} moved and turned"));
            assert!((r.rotation - (20.0 + 0.5f64.to_degrees())).abs() < 1e-6 && (r.width - 80.0).abs() < 1e-6, "{r:?}");
            assert!(r.center.distance(Point::new(140.0, 35.0)) < 1e-6);
            // Scaled along its own axes.
            let along = s.frame() * Affine::scale_non_uniform(1.5, 0.5) * s.frame().inverse();
            let r = s.recover(&p.transformed(along)).unwrap_or_else(|| panic!("{kind:?} scaled"));
            assert!((r.width - 120.0).abs() < 1e-6 && (r.height - 30.0).abs() < 1e-6, "{kind:?} {r:?}");
        }
        // Rounded corners scale uniformly only.
        let s = ParametricShape { radius: 5.0, ..shape(ShapeKind::Star) };
        let p = s.path().unwrap();
        let r = s.recover(&p.transformed(Affine::scale_about(2.0, s.center))).unwrap();
        assert!((r.radius - 10.0).abs() < 1e-9 && (r.width - 160.0).abs() < 1e-9);
        assert_eq!(s.recover(&p.transformed(Affine::scale_non_uniform(2.0, 1.0))), None);
    }

    #[test]
    fn edited_skewed_or_mirrored_shapes_are_not() {
        let s = shape(ShapeKind::Star);
        let p = s.path().unwrap();
        let mut edited = p.clone();
        edited.anchor_mut(0, 3).unwrap().translate(Vec2::new(5.0, 0.0));
        assert_eq!(s.recover(&edited), None);
        assert_eq!(s.recover(&p.transformed(Affine::skew(0.3, 0.0))), None);
        assert_eq!(s.recover(&p.transformed(Affine::FLIP_X)), None);
        let mut more = p.clone();
        more.subpaths[0].insert_anchor(0, 0.5);
        assert_eq!(s.recover(&more), None);
        assert_eq!(s.recover(&PathData::default()), None);
    }

    #[test]
    fn the_fingerprint_follows_the_geometry() {
        let p = shape(ShapeKind::Gear).path().unwrap();
        assert_eq!(fingerprint(&p), fingerprint(&p.clone()));
        assert_eq!(fingerprint(&p), fingerprint(&p.transformed(Affine::translate((1e-5, 0.0)))), "below a thousandth of a point");
        assert_ne!(fingerprint(&p), fingerprint(&p.transformed(Affine::translate((0.01, 0.0)))));
        let mut edited = p.clone();
        edited.anchor_mut(0, 5).unwrap().h_out.x += 1.0;
        assert_ne!(fingerprint(&p), fingerprint(&edited));
        let mut weird = p.clone();
        weird.anchor_mut(0, 0).unwrap().p.x = f64::NAN;
        let _ = fingerprint(&weird);
    }

    #[test]
    fn plain_rectangles_ellipses_and_regular_polygons_are_recognised() {
        use craft_geom::shapes;
        let r = recognise(&shapes::rectangle(kurbo::Rect::new(10.0, 20.0, 110.0, 70.0))).unwrap();
        assert_eq!(r.kind, ShapeKind::Rectangle);
        assert!(r.center.distance(Point::new(60.0, 45.0)) < 1e-9);
        let (w, h) = if r.rotation.abs() < 1e-6 || (r.rotation.abs() - 180.0).abs() < 1e-6 { (r.width, r.height) } else { (r.height, r.width) };
        assert!((w - 100.0).abs() < 1e-9 && (h - 50.0).abs() < 1e-9, "{r:?}");
        let e = recognise(&shapes::ellipse(kurbo::Rect::new(0.0, 0.0, 40.0, 20.0))).unwrap();
        assert_eq!(e.kind, ShapeKind::Ellipse);
        assert!(e.center.distance(Point::new(20.0, 10.0)) < 1e-9);
        let poly = recognise(&shapes::polygon(Point::new(50.0, 50.0), 30.0, 7, 10.0)).unwrap();
        assert_eq!((poly.kind, poly.count), (ShapeKind::Polygon, 7));
        assert!((poly.width - 60.0).abs() < 1e-6);
        // Its outline is the polygon's, if from another first anchor.
        let (a, b) = (poly.path().unwrap().bounds().unwrap(), shapes::polygon(Point::new(50.0, 50.0), 30.0, 7, 10.0).bounds().unwrap());
        assert!(a.center().distance(b.center()) < 1e-9 && (a.area() - b.area()).abs() < 1e-6, "{a:?} {b:?}");
        // Not regular, open, or a star: no.
        assert_eq!(recognise(&PathData::single(SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(3.0, 8.0)], true))), None);
        assert_eq!(recognise(&PathData::single(SubPath::polyline(&[Point::ZERO, Point::new(10.0, 0.0), Point::new(10.0, 10.0)], false))), None);
        assert_eq!(recognise(&shapes::star(Point::ZERO, 10.0, 4.0, 5, 0.0)), None);
    }
}
