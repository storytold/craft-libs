//! The Craft suite's 2D vector drawing kernel (RFC-0030), carved from VectorCraft.
//!
//! The editable path model is anchor based (like every Illustrator tool thinks about paths):
//! a [`PathData`] is a list of [`SubPath`]s, each a list of [`Anchor`]s with absolute in/out
//! handle positions. Rendering and algorithms convert to [`kurbo::BezPath`].
//!
//! Coordinates are document points (1/72 in), y pointing down.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod anchors;
pub mod arc;
pub mod bez;
pub mod corners;
pub mod hit;
pub mod path;
pub mod projective;
pub mod recognize;
pub mod shapes;
pub mod snap;

pub use anchors::Anchors;
pub use arc::ArcPath;
pub use kurbo;
pub use kurbo::{Affine, BezPath, CubicBez, Line, ParamCurve, PathEl, PathSeg, Point, Rect, Shape, Size, Vec2};
pub use path::{Anchor, AnchorKind, FillRule, PathData, SubPath};
pub use projective::Homography;

/// Tolerance used when comparing handle positions to anchor positions.
pub const EPS: f64 = 1e-9;

/// Most line segments [`flatten`] makes of one curve, whatever its size and the tolerance.
pub const MAX_FLATTEN_PER_CURVE: f64 = 4096.0;

/// A curve whose control polygon is longer than this is drawn as a line by [`flatten`]: kurbo
/// squares and cubes coordinates when it sizes its work, and with coordinates this large that
/// overflows to infinity and it reserves a buffer no machine has. (Far beyond any drawing.)
pub const MAX_FLATTEN_LENGTH: f64 = 1e60;

/// The least tolerance [`flatten`] asks kurbo for. kurbo squares its scaled tolerance when it
/// sizes a cubic's subdivision; below this the square underflows to zero, the subdivision count
/// saturates to `usize::MAX` and the buffer reservation panics. (Far below any drawing.)
pub const MIN_FLATTEN_TOLERANCE: f64 = 1e-100;

/// [`kurbo::flatten`], each curve's tolerance raised so that it makes at most about
/// [`MAX_FLATTEN_PER_CURVE`] segments. kurbo sizes its work and a buffer from the curve's
/// coordinates before producing anything, so a small curve with far-out handles (from a file, or
/// scaled up by a view or a projection) asked for gigabytes. The segments a curve needs grow with
/// the square root of its control polygon's length over the tolerance; that ratio is held under
/// the square of the cap. A curve whose points aren't finite is drawn as a line to its end.
pub fn flatten(path: impl IntoIterator<Item = PathEl>, tolerance: f64, mut callback: impl FnMut(PathEl)) {
    let mut last = None;
    let mut start = None;
    for el in path {
        let (from, to, length) = match el {
            PathEl::MoveTo(p) => {
                (last, start) = (Some(p), Some(p));
                callback(el);
                continue;
            }
            PathEl::LineTo(p) => {
                last = Some(p);
                callback(el);
                continue;
            }
            PathEl::ClosePath => {
                last = start;
                callback(el);
                continue;
            }
            PathEl::QuadTo(a, b) => (last, b, last.map(|p| p.distance(a) + a.distance(b))),
            PathEl::CurveTo(a, b, c) => (last, c, last.map(|p| p.distance(a) + a.distance(b) + b.distance(c))),
        };
        last = Some(to);
        let (Some(from), Some(length)) = (from, length) else { continue };
        // A short polygon far from the origin still overflows kurbo's `p * (mt * 2.0)` products.
        let far = |p: Point| !(p.x.abs() <= MAX_FLATTEN_LENGTH && p.y.abs() <= MAX_FLATTEN_LENGTH);
        let controls_far = match el {
            PathEl::QuadTo(a, _) => far(a),
            PathEl::CurveTo(a, b, _) => far(a) || far(b),
            _ => false,
        };
        if length.is_nan() || length > MAX_FLATTEN_LENGTH || far(from) || far(to) || controls_far {
            callback(PathEl::LineTo(to));
            continue;
        }
        let tol = tolerance.max(length / (MAX_FLATTEN_PER_CURVE * MAX_FLATTEN_PER_CURVE)).max(MIN_FLATTEN_TOLERANCE);
        kurbo::flatten([PathEl::MoveTo(from), el], tol, |e| {
            if !matches!(e, PathEl::MoveTo(_)) {
                callback(e);
            }
        });
    }
}

/// Union of two optional rects.
pub fn union_opt(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Normalize an angle in degrees to (-180, 180].
pub fn normalize_deg(mut a: f64) -> f64 {
    a %= 360.0;
    if a <= -180.0 {
        a += 360.0;
    } else if a > 180.0 {
        a -= 360.0;
    }
    a
}

/// Snap a vector to the nearest multiple of `step_deg` (used for Shift-constrained drags).
pub fn constrain_angle(v: Vec2, step_deg: f64) -> Vec2 {
    constrain_angle_from(v, step_deg, 0.0)
}

/// [`constrain_angle`] in steps counted from `base_deg` (the Constrain Angle preference,
/// counter-clockwise as seen on the y-down page).
pub fn constrain_angle_from(v: Vec2, step_deg: f64, base_deg: f64) -> Vec2 {
    let len = v.hypot();
    if len < EPS {
        return v;
    }
    // A whole turn (360°, −720°…) is no turn at all.
    let base_deg = if base_deg.is_finite() { normalize_deg(base_deg) } else { 0.0 };
    let (step, base) = (step_deg.to_radians(), -base_deg.to_radians());
    let a = base + ((v.y.atan2(v.x) - base) / step).round() * step;
    Vec2::new(a.cos() * len, a.sin() * len)
}

/// The page axes turned by `deg` (the Constrain Angle preference, counter-clockwise as seen on
/// the y-down page): maps a vector along the turned axes onto the page. `None` when they are the
/// page's own axes (any multiple of 360°, or a non-finite angle), so callers keep exact page
/// coordinates; multiples of 90° are exact quarter turns, without floating-point noise.
pub fn constrain_axes(deg: f64) -> Option<Affine> {
    if !deg.is_finite() {
        return None;
    }
    let d = normalize_deg(deg);
    let (cos, sin) = if d == 0.0 {
        return None;
    } else if d == 90.0 {
        (0.0, 1.0)
    } else if d == 180.0 {
        (-1.0, 0.0)
    } else if d == -90.0 {
        (0.0, -1.0)
    } else {
        let r = d.to_radians();
        (r.cos(), r.sin())
    };
    // Counter-clockwise on a y-down page is a negative rotation of the coordinates.
    Some(Affine::new([cos, -sin, sin, cos, 0.0, 0.0]))
}

/// A rotation by `deg` (clockwise as seen on the y-down page, as [`Affine::rotate`] takes it in
/// radians), exact at quarter turns and the identity at whole turns; 0 is `Affine::rotate(0.0)`
/// as before, and any other angle is `Affine::rotate` itself.
pub fn rotate_deg(deg: f64) -> Affine {
    if deg.is_finite() && deg != 0.0 {
        let d = normalize_deg(deg);
        if d == 0.0 {
            return Affine::IDENTITY;
        }
        if d % 90.0 == 0.0
            && let Some(a) = constrain_axes(-d)
        {
            return a;
        }
    }
    Affine::rotate(deg.to_radians())
}

/// The `rotation` (degrees, as the shape and type commands take it) that turns a new object from
/// the page axes onto the axes of Constrain Angle `base`; `None` for the page's own axes (0°, a
/// whole turn, a non-finite angle), where new objects are drawn exactly as without it.
pub fn constrain_rotation(base: f64) -> Option<f64> {
    constrain_axes(base).map(|_| -normalize_deg(base))
}

/// Rect from two corner points in any order.
pub fn rect_from_points(a: Point, b: Point) -> Rect {
    Rect::from_points(a, b)
}

/// The 9 reference points of a rect (Illustrator's reference-point locator), row-major from top-left.
pub fn reference_point(r: Rect, index: usize) -> Point {
    let xs = [r.x0, r.center().x, r.x1];
    let ys = [r.y0, r.center().y, r.y1];
    Point::new(xs[index % 3], ys[(index / 3).min(2)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny curve with far-out handles flattens into a bounded number of segments; an ordinary
    /// one exactly as kurbo flattens it.
    #[test]
    fn flatten_bounds_the_segments_of_one_curve() {
        let count = |path: &[PathEl], tol: f64| {
            let mut n = 0usize;
            flatten(path.iter().copied(), tol, |_| n += 1);
            n
        };
        let wild = [PathEl::MoveTo(Point::ZERO), PathEl::CurveTo(Point::new(1e15, -1e15), Point::new(-1e15, 1e15), Point::new(1.0, 0.0))];
        assert!(count(&wild, 0.25) < 3 * MAX_FLATTEN_PER_CURVE as usize);
        let plain =
            [PathEl::MoveTo(Point::ZERO), PathEl::CurveTo(Point::new(0.0, 50.0), Point::new(100.0, 50.0), Point::new(100.0, 0.0)), PathEl::ClosePath];
        let mut ours = vec![];
        flatten(plain.iter().copied(), 0.25, |e| ours.push(e));
        let mut theirs = vec![];
        kurbo::flatten(plain.iter().copied(), 0.25, |e| theirs.push(e));
        assert_eq!(ours, theirs);
        let nan = [PathEl::MoveTo(Point::ZERO), PathEl::QuadTo(Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0))];
        assert_eq!(count(&nan, 0.25), 2);
        // Finite coordinates whose squares overflow are a line to the end too.
        let huge = [PathEl::MoveTo(Point::ZERO), PathEl::CurveTo(Point::new(1e200, 0.0), Point::new(-1e200, 1e200), Point::new(1.0, 0.0))];
        assert_eq!(count(&huge, 0.25), 2);
        // A short polygon out near f64::MAX: a line, and every point it emits is finite.
        let m = f64::MAX / 2.0;
        let far = [PathEl::MoveTo(Point::new(m, m)), PathEl::CurveTo(Point::new(m, m), Point::new(m, m), Point::new(m, m))];
        let mut points = vec![];
        flatten(far.iter().copied(), 0.25, |e| points.push(e));
        assert_eq!(points.len(), 2);
        assert!(points.iter().all(|e| matches!(e, PathEl::MoveTo(p) | PathEl::LineTo(p) if p.is_finite())), "{points:?}");
    }

    #[test]
    fn constrain_axes_are_exact_and_whole_turns_are_none() {
        for d in [0.0, 360.0, -360.0, 720.0, f64::NAN, f64::INFINITY] {
            assert!(constrain_axes(d).is_none(), "{d}");
        }
        let v = Point::new(3.0, 4.0);
        // 90° counter-clockwise on the page: x points up (−y).
        assert_eq!(constrain_axes(90.0).unwrap() * Point::new(1.0, 0.0), Point::new(0.0, -1.0));
        assert_eq!(constrain_axes(450.0).unwrap() * v, constrain_axes(90.0).unwrap() * v);
        assert_eq!(constrain_axes(-90.0).unwrap() * v, Point::new(-4.0, 3.0));
        assert_eq!(constrain_axes(180.0).unwrap() * v, Point::new(-3.0, -4.0));
        assert_eq!(constrain_axes(-180.0).unwrap() * v, Point::new(-3.0, -4.0));
        let a = constrain_axes(30.0).unwrap() * Point::new(1.0, 0.0);
        assert!((a.x - 30f64.to_radians().cos()).abs() < 1e-12 && (a.y + 0.5).abs() < 1e-12, "{a:?}");
        assert_eq!(constrain_angle_from(Vec2::new(5.0, 1.0), 45.0, 360.0), constrain_angle_from(Vec2::new(5.0, 1.0), 45.0, 0.0));
    }

    #[test]
    fn rotate_deg_is_exact_at_quarter_turns() {
        let v = Point::new(3.0, 4.0);
        assert_eq!(rotate_deg(90.0) * v, Point::new(-4.0, 3.0));
        assert_eq!(rotate_deg(-90.0) * v, Point::new(4.0, -3.0));
        assert_eq!(rotate_deg(180.0) * v, Point::new(-3.0, -4.0));
        assert_eq!(rotate_deg(-270.0) * v, rotate_deg(90.0) * v);
        assert_eq!(rotate_deg(360.0) * v, v);
        assert_eq!(rotate_deg(0.0), Affine::rotate(0.0));
        assert_eq!(rotate_deg(30.0), Affine::rotate(30f64.to_radians()));
        assert_eq!(constrain_rotation(30.0), Some(-30.0));
        assert_eq!(constrain_rotation(0.0), None);
        assert_eq!(constrain_rotation(-360.0), None);
        assert_eq!(constrain_rotation(450.0), Some(-90.0));
    }

    #[test]
    fn angle_normalization() {
        assert_eq!(normalize_deg(190.0), -170.0);
        assert_eq!(normalize_deg(-190.0), 170.0);
        assert_eq!(normalize_deg(180.0), 180.0);
        assert_eq!(normalize_deg(720.0), 0.0);
    }

    #[test]
    fn constrain_45() {
        let v = constrain_angle(Vec2::new(10.0, 1.0), 45.0);
        assert!((v.y).abs() < 1e-9);
        let v = constrain_angle(Vec2::new(10.0, 9.0), 45.0);
        assert!((v.x - v.y).abs() < 1e-9);
    }

    #[test]
    fn constrain_from_a_base_angle() {
        // 30° base: 45° steps land on 30°, 75°, … (counter-clockwise on the page, y down).
        let deg = |v: Vec2| (-v.y).atan2(v.x).to_degrees();
        assert!((deg(constrain_angle_from(Vec2::new(10.0, -4.0), 45.0, 30.0)) - 30.0).abs() < 1e-9);
        assert!((deg(constrain_angle_from(Vec2::new(3.0, -10.0), 45.0, 30.0)) - 75.0).abs() < 1e-9);
        assert!((deg(constrain_angle_from(Vec2::new(10.0, 1.0), 45.0, 30.0)) + 15.0).abs() < 1e-9);
    }

    #[test]
    fn reference_points() {
        let r = Rect::new(0.0, 0.0, 10.0, 20.0);
        assert_eq!(reference_point(r, 0), Point::new(0.0, 0.0));
        assert_eq!(reference_point(r, 4), Point::new(5.0, 10.0));
        assert_eq!(reference_point(r, 8), Point::new(10.0, 20.0));
    }

    #[test]
    fn union_options() {
        let a = Rect::new(0.0, 0.0, 1.0, 1.0);
        let b = Rect::new(2.0, 2.0, 3.0, 3.0);
        assert_eq!(union_opt(Some(a), Some(b)), Some(Rect::new(0.0, 0.0, 3.0, 3.0)));
        assert_eq!(union_opt(None, Some(b)), Some(b));
        assert_eq!(union_opt(None, None), None);
    }

    /// A tiny curve must not panic however small the tolerance: kurbo's subdivision count
    /// saturates when its scaled tolerance underflows, and the buffer reservation would overflow.
    #[test]
    fn flatten_survives_underflowing_tolerances() {
        let tiny = [
            PathEl::MoveTo(Point::new(1e-160, 0.0)),
            PathEl::CurveTo(Point::new(2e-160, 1e-160), Point::new(3e-160, -1e-160), Point::new(4e-160, 0.0)),
        ];
        for tol in [0.0, f64::NAN, f64::MIN_POSITIVE, 1e-167] {
            let mut n = 0usize;
            flatten(tiny.iter().copied(), tol, |e| {
                n += 1;
                assert!(matches!(e, PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) if p.is_finite()));
            });
            assert!(n >= 2 && n < 3 * MAX_FLATTEN_PER_CURVE as usize, "{n} at {tol}");
        }
    }
}
