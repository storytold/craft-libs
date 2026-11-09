#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

#[macro_use]
mod typed_vec;

#[cfg(any(test, feature = "arbitrary"))]
pub mod arbitrary;
pub mod curve;
mod geom;
mod num;
pub mod order;
mod position;
mod segments;
pub mod sweep;
pub mod topology;

#[cfg(feature = "generators")]
pub mod generators;

pub use geom::{Point, Segment};
use kurbo::Shape;
pub use segments::{SegIdx, Segments};

// pub so that we can use it in fuzz tests, but it's really private
#[doc(hidden)]
pub mod treevec;

use topology::{BinaryWindingNumber, Topology};

use crate::segments::NonClosedPath;

#[cfg(test)]
pub mod perturbation;

/// A fill rule tells us how to decide whether a point is "inside" a polyline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FillRule {
    /// The point is "inside" if its winding number is odd.
    EvenOdd,
    /// The point is "inside" if its winding number is non-zero.
    NonZero,
}

impl FillRule {
    /// According to this fill rule, is the winding number inside the shape?
    pub fn is_inside(self, winding: i32) -> bool {
        match self {
            FillRule::EvenOdd => winding % 2 != 0,
            FillRule::NonZero => winding != 0,
        }
    }
}

/// Binary operations between sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum BinaryOp {
    /// A point is in the union of two sets if it is in either one.
    Union,
    /// A point is in the intersection of two sets if it is in both.
    Intersection,
    /// A point is in the difference of two sets if it is in the first but not the second.
    Difference,
    /// A point is in the exclusive-or of two sets if it is in one or the other, but not both.
    Xor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// The input points were faulty.
pub enum Error {
    /// At least one of the inputs was infinite.
    Infinity,
    /// At least one of the inputs was not a number.
    NaN,
    /// One of the inputs had a non-closed path.
    NonClosedPath(NonClosedPath),
    /// The sweep gave winding numbers that disagree around a point, so the
    /// contours of the output set could not be walked.
    InconsistentWindingNumbers,
    /// At least one of the inputs had a coordinate with a magnitude above [`MAX_COORDINATE`].
    TooLarge,
    /// The sweep did more work (segment comparisons, position checks and linear searches of the
    /// sweep line) than its work limit allows.
    WorkLimitExceeded,
    /// The accuracy parameter was NaN, infinite, zero or negative.
    InvalidEps,
    /// [`binary_op`] chose an accuracy parameter `eps` from the largest
    /// coordinate of the inputs, and one input subpath is narrower than
    /// [`FEATURE_EPS_RATIO`] times `eps` in x or y. The sweep could merge
    /// that subpath away or drop it, while the smallest automatic `eps` would
    /// keep it. Call [`binary_op_with_eps`] to choose `eps` yourself.
    // Craft patch (far-point eps).
    FeatureBelowTolerance {
        /// The accuracy parameter that `binary_op` chose.
        eps: f64,
    },
}

/// The largest coordinate magnitude that [`Topology`] and [`binary_op`] accept.
///
/// The curve arithmetic overflows for coordinates above about `f64::MAX / 3`.
pub const MAX_COORDINATE: f64 = f64::MAX / 16.0;

/// Checks that every point of `path` is finite and at most [`MAX_COORDINATE`] in magnitude.
pub(crate) fn check_coordinates(path: &kurbo::BezPath) -> Result<(), Error> {
    for el in path.iter() {
        let (p0, p1, p2) = match el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => (p, p, p),
            kurbo::PathEl::QuadTo(p0, p1) => (p0, p1, p1),
            kurbo::PathEl::CurveTo(p0, p1, p2) => (p0, p1, p2),
            kurbo::PathEl::ClosePath => continue,
        };
        for v in [p0.x, p0.y, p1.x, p1.y, p2.x, p2.y] {
            if v.is_nan() {
                return Err(Error::NaN);
            }
            if v.is_infinite() {
                return Err(Error::Infinity);
            }
            if v.abs() > MAX_COORDINATE {
                return Err(Error::TooLarge);
            }
        }
    }
    Ok(())
}

impl From<NonClosedPath> for Error {
    fn from(ncp: NonClosedPath) -> Self {
        Error::NonClosedPath(ncp)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Infinity => write!(f, "one of the inputs was infinite"),
            Error::NaN => write!(f, "one of the inputs had a NaN"),
            Error::NonClosedPath(_) => write!(f, "one of the inputs had a non-closed path"),
            Error::InconsistentWindingNumbers => {
                write!(f, "the winding numbers disagree around a point")
            }
            Error::TooLarge => write!(f, "one of the inputs had a coordinate that was too large"),
            Error::WorkLimitExceeded => {
                write!(f, "the sweep did more work than its work limit")
            }
            Error::InvalidEps => {
                write!(f, "the accuracy parameter was not a positive finite number")
            }
            // Craft patch (far-point eps).
            Error::FeatureBelowTolerance { eps } => write!(
                f,
                "one of the inputs had a subpath too small for the accuracy {eps} that its largest coordinate needs"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Computes a boolean operation between two sets, each of which is described as
/// a collection of closed curves.
///
/// This function calls [`binary_op_with_eps`] with an automatically-chosen
/// accuracy parameter. The automatically-chosen accuracy is fairly
/// conservative, so you may get better performance by choosing your own.
///
/// The accuracy parameter is `64 * f64::EPSILON` times the largest coordinate
/// magnitude of either input, and at least [`MIN_AUTO_EPS`]. The sweep uses it
/// for every segment, so one far point coarsens the result everywhere. When it
/// is above [`MIN_AUTO_EPS`] and an input subpath that is at least
/// [`FEATURE_EPS_RATIO`] times [`MIN_AUTO_EPS`] wide and high is narrower than
/// [`FEATURE_EPS_RATIO`] times the accuracy parameter in x or y, this function
/// returns [`Error::FeatureBelowTolerance`] instead of a result without that
/// subpath. The check uses each subpath's bounding box, so a thin part of a
/// larger subpath, or a gap narrower than the accuracy parameter between two
/// subpaths, can still be merged without an error.
// Craft patch (far-point eps): the doc paragraph above and the check below.
pub fn binary_op(
    set_a: &kurbo::BezPath,
    set_b: &kurbo::BezPath,
    fill_rule: FillRule,
    op: BinaryOp,
) -> Result<topology::Contours, Error> {
    // Find the extremal values, to figure out how much precision we can support.
    // Craft patch (leading ClosePath): `BezPath::bounding_box` panics on a leading ClosePath.
    let bounding_box = |path: &kurbo::BezPath| {
        if matches!(path.elements().first(), Some(kurbo::PathEl::ClosePath)) {
            kurbo::segments(segments::skip_leading_close_paths(path.iter()))
                .map(|seg| kurbo::ParamCurveExtrema::bounding_box(&seg))
                .reduce(|a, b| a.union(b))
                .unwrap_or_default()
        } else {
            path.bounding_box()
        }
    };
    let bbox = bounding_box(set_a).union(bounding_box(set_b));
    let min = bbox.min_x().min(bbox.min_y());
    let max = bbox.max_x().max(bbox.max_y());
    if min.is_infinite() || max.is_infinite() {
        return Err(Error::Infinity);
    }
    // If there was any NaN in the input, it should have propagated to the min and max.
    if min.is_nan() || max.is_nan() {
        return Err(Error::NaN);
    }

    // TODO: we did some analysis for error bounds in the case of polylines.
    // Think more about what makes sense for curves.
    let m_2 = min.abs().max(max.abs());
    let eps = m_2 * (f64::EPSILON * 64.0);
    let eps = eps.max(MIN_AUTO_EPS);

    debug_assert!(eps.is_finite());
    // Craft patch (far-point eps).
    if eps > MIN_AUTO_EPS {
        let kept = MIN_AUTO_EPS * FEATURE_EPS_RATIO;
        let limit = eps * FEATURE_EPS_RATIO;
        let too_small = [set_a, set_b]
            .into_iter()
            .flat_map(subpath_bounding_boxes)
            .map(|rect| rect.width().min(rect.height()))
            .any(|size| size >= kept && size < limit);
        if too_small {
            return Err(Error::FeatureBelowTolerance { eps });
        }
    }
    binary_op_with_eps(set_a, set_b, fill_rule, op, eps)
}

/// The smallest accuracy parameter that [`binary_op`] chooses.
// Craft patch (far-point eps).
pub const MIN_AUTO_EPS: f64 = 1e-6;

/// [`binary_op`] returns [`Error::FeatureBelowTolerance`] for an input
/// subpath narrower than this many times its accuracy parameter in x or y.
/// The sweep widens each segment by `eps` on both sides, and the output can
/// move by up to `eps`.
// Craft patch (far-point eps).
pub const FEATURE_EPS_RATIO: f64 = 4.0;

/// The exact bounding box of each subpath of `path`, with its closing segment.
// Craft patch (far-point eps).
fn subpath_bounding_boxes(path: &kurbo::BezPath) -> Vec<kurbo::Rect> {
    use kurbo::{CubicBez, Line, ParamCurve, ParamCurveExtrema, PathEl, PathSeg, QuadBez, Rect};

    let mut boxes = Vec::new();
    let mut current: Option<Rect> = None;
    let mut start = kurbo::Point::ZERO;
    let mut last = start;
    for el in path.elements() {
        let seg = match *el {
            PathEl::MoveTo(p) => {
                boxes.extend(current.take());
                start = p;
                last = p;
                current = Some(Rect::from_points(p, p));
                continue;
            }
            PathEl::LineTo(p) => PathSeg::Line(Line::new(last, p)),
            PathEl::QuadTo(p1, p2) => PathSeg::Quad(QuadBez::new(last, p1, p2)),
            PathEl::CurveTo(p1, p2, p3) => PathSeg::Cubic(CubicBez::new(last, p1, p2, p3)),
            PathEl::ClosePath => PathSeg::Line(Line::new(last, start)),
        };
        last = seg.end();
        let rect = ParamCurveExtrema::bounding_box(&seg);
        current = Some(current.map_or(rect, |r| r.union(rect)));
    }
    boxes.extend(current);
    boxes
}

/// Computes a boolean operation between two sets, each of which is described as
/// a collection of closed curves.
///
/// The output paths will not exactly agree with the input paths in general.
/// The accuracy parameter `eps` bounds how much they can differ.
///
/// Returns [`Error::InvalidEps`] when `eps` is NaN, infinite, zero or
/// negative. An `eps` below `64 * f64::EPSILON` times the largest coordinate
/// magnitude is raised to that value.
///
/// `eps` is one absolute tolerance for every segment, so a part of the input
/// narrower than about `eps`, or a gap narrower than about `eps`, can be
/// merged away or dropped.
// Craft patch (far-point eps): the doc paragraph above.
pub fn binary_op_with_eps(
    set_a: &kurbo::BezPath,
    set_b: &kurbo::BezPath,
    fill_rule: FillRule,
    op: BinaryOp,
    eps: f64,
) -> Result<topology::Contours, Error> {
    if !(eps.is_finite() && eps > 0.0) {
        return Err(Error::InvalidEps);
    }
    let top = Topology::from_paths_binary(set_a, set_b, eps)?;
    #[cfg(feature = "debug-svg")]
    {
        svg::save(
            "out.svg",
            &top.dump_svg(|tag| {
                if tag == topology::OneOfTwo::A {
                    "red".to_owned()
                } else {
                    "blue".to_owned()
                }
            }),
        )
        .unwrap();
    }

    let inside = |windings: &BinaryWindingNumber| {
        let inside_one = |winding| match fill_rule {
            FillRule::EvenOdd => winding % 2 != 0,
            FillRule::NonZero => winding != 0,
        };

        match op {
            BinaryOp::Union => inside_one(windings.shape_a) || inside_one(windings.shape_b),
            BinaryOp::Intersection => inside_one(windings.shape_a) && inside_one(windings.shape_b),
            BinaryOp::Xor => inside_one(windings.shape_a) != inside_one(windings.shape_b),
            BinaryOp::Difference => inside_one(windings.shape_a) && !inside_one(windings.shape_b),
        }
    };

    top.try_contours(inside)
}

#[cfg(test)]
mod tests {
    use insta::assert_ron_snapshot;
    use kurbo::BezPath;

    use super::*;

    #[test]
    fn two_squares() {
        fn to_bez(mut points: impl Iterator<Item = (f64, f64)>) -> BezPath {
            let p = points.next().unwrap();
            let mut ret = BezPath::default();
            ret.move_to(p);
            for q in points {
                ret.line_to(q);
            }
            ret.line_to(p);
            ret
        }
        let a = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let b = vec![(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
        let output = binary_op(
            &to_bez(a.into_iter()),
            &to_bez(b.into_iter()),
            FillRule::EvenOdd,
            BinaryOp::Intersection,
        )
        .unwrap();

        insta::assert_ron_snapshot!(output);
    }

    // Craft patch (leading ClosePath).
    #[test]
    fn leading_close_path() {
        // `BezPath`'s own constructors reject these, but deserializing does not.
        let close_only: BezPath = serde_json::from_str(r#"["ClosePath"]"#).unwrap();
        let triangle = r#"{"MoveTo":{"x":0.0,"y":0.0}},{"LineTo":{"x":10.0,"y":0.0}},{"LineTo":{"x":0.0,"y":10.0}},"ClosePath""#;
        let plain: BezPath = serde_json::from_str(&format!("[{triangle}]")).unwrap();
        let leading: BezPath =
            serde_json::from_str(&format!(r#"["ClosePath",{triangle}]"#)).unwrap();

        let empty = Topology::<i32>::from_path(&close_only, 1e-6).unwrap();
        assert_eq!(empty.contours(|w| *w != 0).contours().count(), 0);

        let expected = Topology::<i32>::from_path(&plain, 1e-6)
            .unwrap()
            .contours(|w| *w != 0);
        let actual = Topology::<i32>::from_path(&leading, 1e-6)
            .unwrap()
            .contours(|w| *w != 0);
        let paths =
            |c: &topology::Contours| c.contours().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&actual), paths(&expected));
        assert_eq!(paths(&actual).len(), 1);

        for (a, b) in [
            (&close_only, &plain),
            (&plain, &close_only),
            (&leading, &plain),
            (&plain, &leading),
        ] {
            let output = binary_op(a, b, FillRule::NonZero, BinaryOp::Union).unwrap();
            assert_eq!(output.contours().count(), 1);
        }
    }

    // This example has a union of two not-quite-axis-aligned crosses. It
    // suffers from some extra quadratic segments near the intersection points,
    // but it used to be worse.
    #[test]
    fn path_blowup() {
        let path1 = "M-90.03662872314453,-212 L-90.03662872314453,212 L90.03565216064453,212 L90.03565216064453,-212 L-90.03662872314453,-212 Z";
        let path2 = "M211.99964904785156,-90.03582000732422 L-212.00035095214844,-90.03646087646484 L-212.00062561035156,90.03582000732422 L211.99937438964844,90.03646087646484 L211.99964904785156,-90.03582000732422 Z";
        if let Ok(output) = binary_op(
            &BezPath::from_svg(path1).unwrap(),
            &BezPath::from_svg(path2).unwrap(),
            FillRule::NonZero,
            BinaryOp::Union,
        ) {
            let output = output.contours().next().unwrap();
            let path_length = output.path.elements().len();
            // This should ideally be more like 12, but there
            // are some small horizontal segments in the output.
            // It used to be 77, though...
            assert!(path_length <= 16);
        }
    }

    #[test]
    fn test_empty_horizontals() {
        let square = kurbo::Rect::from_origin_size((0.0, 0.0), (10.0, 10.0)).to_path(0.0);
        let line_and_back = {
            let mut path = kurbo::Line::new((-1.0, 0.0), (1.0, 0.0)).to_path(0.0);
            path.close_path();
            path
        };
        let line_and_back_2 = {
            let mut path = kurbo::Line::new((2.0, 1.0), (12.0, 1.0)).to_path(0.0);
            path.close_path();
            path
        };

        Topology::from_path(&line_and_back, 1e-3).unwrap();
        Topology::from_path(&line_and_back_2, 1e-3).unwrap();
        Topology::<i32>::from_paths([(&line_and_back, ()), (&line_and_back_2, ())], 1e-3).unwrap();
        Topology::<i32>::from_paths(
            [(&line_and_back, ()), (&line_and_back_2, ()), (&square, ())],
            1e-3,
        )
        .unwrap();

        let output = binary_op(
            &square,
            &line_and_back_2,
            FillRule::EvenOdd,
            BinaryOp::Difference,
        )
        .unwrap();
        assert_ron_snapshot!(output);
    }

    #[test]
    fn monotonicity_violation() {
        let path = "M-16.252847470703124,8.977497100830078 C-8.126453262439954,8.977497100830078 -3.724613933559983,8.977497100830078 4.401851380682135,8.977497100830078 C4.401851380682135,8.977497100830078 16.252847872314454,8.977497100830075 16.252847872314454,-0.5280563794708262 C16.252847872314454,-8.977497100830078 4.401851380682135,-8.977497100830075 4.401851380682135,-8.977497100830075 L4.401851380682135,-8.977497100830075 C-5.078945812623724,-8.977497100830075 -1.5236468651340243,-8.977497100830075 -11.004562568404802,-8.977497100830075 Z";
        let path = BezPath::from_svg(path).unwrap();
        Topology::from_path(&path, 0.001).unwrap();
    }

    // Craft patch (add_bez_path error).
    #[test]
    fn add_bez_path_error_leaves_segments_unchanged() {
        let mut segments = Segments::default();
        let open = BezPath::from_svg("M0,0 L10,0 L5,10 Z M20,0 L30,5 L25,10").unwrap();
        assert!(segments.add_bez_path(&open).is_err());
        assert_eq!(segments.len(), 0);
        segments.check_invariants();

        let square = BezPath::from_svg("M40,0 L50,0 L50,10 L40,10 Z").unwrap();
        segments.add_bez_path(&square).unwrap();
        segments.check_invariants();
        assert_eq!(segments.len(), 4);
        let mut entered: Vec<_> = segments.entrances().iter().map(|(_, idx)| *idx).collect();
        entered.sort();
        assert_eq!(entered, segments.indices().collect::<Vec<_>>());
    }

    #[test]
    fn empty_paths() {
        let contours = binary_op(
            &BezPath::new(),
            &BezPath::new(),
            FillRule::EvenOdd,
            BinaryOp::Xor,
        )
        .unwrap();
        assert!(contours.contours().next().is_none());
    }

    #[test]
    fn inconsistent_winding_numbers_are_an_error() {
        // The sweep gives this path winding numbers that disagree around a
        // point.
        let path = BezPath::from_svg("M4,-1 C8,1 -7,-2 5,8 L-4,-8 L2,1 L1,1 Z").unwrap();
        let result = binary_op(&path, &BezPath::new(), FillRule::EvenOdd, BinaryOp::Union);
        assert_eq!(result.err(), Some(Error::InconsistentWindingNumbers));
    }

    #[test]
    fn repeated_points() {
        Topology::from_polylines_binary(
            [[
                Point::new(0.0, 0.0),
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(1.0, 1.0),
            ]],
            [[
                Point::new(0.0, 0.0),
                Point::new(0.0, 1.0),
                Point::new(1.0, 1.0),
            ]],
            1e-6,
        );
    }

    #[test]
    fn quadratics_in_quadratics_out() {
        let mut quad_path = BezPath::new();
        quad_path.move_to((0.0, 0.0));
        quad_path.quad_to((1.0, -5.0), (2.0, 0.0));
        quad_path.quad_to((1.0, 5.0), (0.0, 0.0));

        let mut rectangle = BezPath::new();
        rectangle.move_to((-1.0, -1.0));
        rectangle.line_to((-1.0, 1.0));
        rectangle.line_to((3.0, 1.0));
        rectangle.line_to((3.0, -1.0));
        rectangle.line_to((-1.0, -1.0));

        let mut segs = Segments::default();
        segs.add_bez_path(&quad_path).unwrap();

        let contours = binary_op(
            &quad_path,
            &rectangle,
            FillRule::EvenOdd,
            BinaryOp::Intersection,
        )
        .unwrap();
        assert_eq!(contours.contours().count(), 1);
        let contour = contours.contours().next().unwrap();
        assert_eq!(
            contour
                .path
                .elements()
                .iter()
                .filter(|el| matches!(el, &&kurbo::PathEl::QuadTo(..)))
                .count(),
            4
        );
    }

    #[test]
    fn coordinates_above_max_coordinate() {
        let shape = |y: f64| {
            let mut path = BezPath::new();
            path.move_to((0.0, 0.0));
            path.curve_to((y / 4.0, y), (y / 2.0, y), (y, y));
            path.line_to((0.0, y));
            path.close_path();
            path
        };

        assert_eq!(check_coordinates(&shape(MAX_COORDINATE)), Ok(()));
        let above = f64::from_bits(MAX_COORDINATE.to_bits() + 1);
        assert_eq!(check_coordinates(&shape(above)), Err(Error::TooLarge));
        assert_eq!(check_coordinates(&shape(-above)), Err(Error::TooLarge));
        assert_eq!(
            check_coordinates(&shape(f64::INFINITY)),
            Err(Error::Infinity)
        );
        assert_eq!(check_coordinates(&shape(f64::NAN)), Err(Error::NaN));

        for y in [6e307, 7e307, 1.7e308] {
            let path = shape(y);
            let top = Topology::<i32>::from_path(&path, y * f64::EPSILON * 64.0);
            assert!(matches!(top, Err(Error::TooLarge)));
            let op = binary_op(&path, &path, FillRule::NonZero, BinaryOp::Union);
            assert!(matches!(op, Err(Error::TooLarge)));
        }
    }

    /// Two unit circles whose centers are one unit apart, at `(offset, offset)`
    /// and `(offset + 1, offset)`.
    fn crossing_circles(offset: f64) -> (BezPath, BezPath) {
        (
            kurbo::Circle::new((offset, offset), 1.0).to_path(1e-9),
            kurbo::Circle::new((offset + 1.0, offset), 1.0).to_path(1e-9),
        )
    }

    /// The single contour's area, measured after moving it back to the origin.
    fn single_contour_area(contours: &topology::Contours, offset: f64) -> f64 {
        let contours: Vec<_> = contours.contours().collect();
        assert_eq!(contours.len(), 1);
        (kurbo::Affine::translate((-offset, -offset)) * contours[0].path.clone())
            .area()
            .abs()
    }

    const UNION_AREA: f64 = 5.054816;

    #[test]
    fn binary_op_with_eps_rejects_invalid_eps() {
        let (a, b) = crossing_circles(0.0);
        // -inf goes last: before the check it made the sweep loop forever.
        for eps in [
            f64::NAN,
            -1.0,
            -1e-6,
            0.0,
            -0.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let result = binary_op_with_eps(&a, &b, FillRule::NonZero, BinaryOp::Union, eps);
            assert_eq!(result.err(), Some(Error::InvalidEps), "eps = {eps}");
        }
    }

    #[test]
    fn union_with_huge_coordinates() {
        use kurbo::{Affine, Circle, Rect};

        // A circle overlapping a rectangle. The union is one contour whose area is
        // s^2 * (pi / 9 + 4 / 9 - pi / 36), give or take the circle's flattening.
        let union_area = |s: f64| {
            let r = s / 3.0;
            let circle = Circle::new((r, r), r).to_path(1e-3 * r);
            let rect = Rect::new(r, r, s, s).to_path(0.0);
            let contours = binary_op(&circle, &rect, FillRule::NonZero, BinaryOp::Union).unwrap();
            let paths: Vec<_> = contours.contours().map(|c| c.path.clone()).collect();
            let area: f64 = paths
                .iter()
                .map(|p| (Affine::scale(1.0 / s) * p.clone()).area())
                .sum();
            (paths.len(), area.abs())
        };

        let (count, expected) = union_area(1.0);
        assert_eq!(count, 1);
        for s in [1e155, 1e200, 1e300] {
            let (count, area) = union_area(s);
            assert_eq!(count, 1, "s = {s}");
            assert!(
                (area - expected).abs() < 1e-4,
                "s = {s}: {area} != {expected}"
            );
        }
    }

    #[test]
    fn topology_raises_unusable_eps() {
        let inside = |w: &BinaryWindingNumber| w.shape_a != 0 || w.shape_b != 0;
        // At 1.5e8 one ulp is about 3e-8, so 1e-10 used to recurse in
        // intersect_cubics_rec until the stack overflowed.
        for (offset, eps) in [
            (0.0, f64::NAN),
            (0.0, -1.0),
            (0.0, 0.0),
            (1.5e8, 1e-10),
            (1.5e8, 0.0),
            (0.0, f64::NEG_INFINITY),
            (0.0, f64::INFINITY),
        ] {
            let (a, b) = crossing_circles(offset);
            for correctness in [
                sweep::Correctness::GoodEnough,
                sweep::Correctness::ReallyCorrect,
            ] {
                let top = Topology::from_paths_with_correctness(
                    [(&a, topology::OneOfTwo::A), (&b, topology::OneOfTwo::B)],
                    eps,
                    correctness,
                )
                .unwrap();
                let area = single_contour_area(&top.contours(inside), offset);
                assert!(
                    (area - UNION_AREA).abs() < 1e-3,
                    "offset {offset}, eps {eps}, {correctness:?}: area {area}"
                );
            }
        }
    }

    #[test]
    fn binary_op_with_eps_raises_eps_below_coordinate_resolution() {
        let (a, b) = crossing_circles(1.5e8);
        let contours =
            binary_op_with_eps(&a, &b, FillRule::NonZero, BinaryOp::Union, 1e-10).unwrap();
        let area = single_contour_area(&contours, 1.5e8);
        assert!((area - UNION_AREA).abs() < 1e-3, "area {area}");
    }

    // Craft patch (far-point eps).
    #[test]
    fn far_point_eps() {
        let rect = |x0, y0, x1, y1| kurbo::Rect::new(x0, y0, x1, y1).to_path(0.0);
        let far_triangle = |at: f64| {
            let mut path = BezPath::new();
            path.move_to((at, at));
            path.line_to((at + 1.0e3, at));
            path.line_to((at, at + 1.0e3));
            path.close_path();
            path
        };
        let with = |base: &BezPath, extra: &BezPath| {
            let mut path = base.clone();
            path.extend(extra.iter());
            path
        };
        let auto_eps = |far: f64| ((far + 1.0e3) * (f64::EPSILON * 64.0)).max(MIN_AUTO_EPS);
        let union = |a: &BezPath, b: &BezPath| binary_op(a, b, FillRule::NonZero, BinaryOp::Union);
        let inside = |contours: &topology::Contours, p: (f64, f64)| {
            contours
                .contours()
                .map(|c| c.path.winding(p.into()))
                .sum::<i32>()
                != 0
        };

        // A 10 x 10 square and a 0.3 x 10 sliver, united with another square.
        let near = with(&rect(0.0, 0.0, 10.0, 10.0), &rect(40.0, 0.0, 40.3, 10.0));
        let other = rect(5.0, 5.0, 15.0, 15.0);
        let out = union(&near, &other).unwrap();
        assert!(inside(&out, (40.15, 5.0)));
        assert!(inside(&out, (2.0, 2.0)));

        // A triangle at 3e13 raises eps to about 0.43, and the sliver is
        // narrower than 4 * eps.
        let far = 3.0e13;
        assert_eq!(
            union(&with(&near, &far_triangle(far)), &other).unwrap_err(),
            Error::FeatureBelowTolerance { eps: auto_eps(far) }
        );
        assert_eq!(
            union(&other, &with(&near, &far_triangle(far))).unwrap_err(),
            Error::FeatureBelowTolerance { eps: auto_eps(far) }
        );

        // A triangle at 1e12 gives eps about 0.014, and nothing is that small.
        let out = union(&with(&near, &far_triangle(1.0e12)), &other).unwrap();
        assert!(inside(&out, (40.15, 5.0)));

        // Every subpath at least 4 * eps wide and high: no error at eps about 14.
        let far = 1.0e15;
        let big = with(&rect(0.0, 0.0, 100.0, 100.0), &far_triangle(far));
        assert!(union(&big, &rect(200.0, 0.0, 300.0, 100.0)).is_ok());

        // A zero-width subpath is lost at any eps, so it is not reported.
        let mut line_and_back = kurbo::Line::new((0.0, 50.0), (100.0, 50.0)).to_path(0.0);
        line_and_back.close_path();
        assert!(union(&with(&big, &line_and_back), &BezPath::new()).is_ok());

        // A subpath below 4 * MIN_AUTO_EPS is lost at any eps, so it is not reported.
        let tiny = rect(50.0, 50.0, 50.0 + 1.0e-7, 50.0 + 1.0e-7);
        assert!(union(&with(&big, &tiny), &BezPath::new()).is_ok());
        assert!(union(&tiny, &far_triangle(1.0e8)).is_ok());

        // The check measures curves, not control points: this cubic's control
        // points span 10 in x and the curve about 2.9, below 4 * eps (about 3.98).
        let mut thin_curve = BezPath::new();
        thin_curve.move_to((40.0, 0.0));
        thin_curve.curve_to((45.0, 10.0 / 3.0), (35.0, 20.0 / 3.0), (40.0, 10.0));
        thin_curve.close_path();
        let width = subpath_bounding_boxes(&thin_curve)[0].width();
        assert!(width > 2.8 && width < 2.9, "{width}");
        let far = 7.0e13;
        assert!(union(&thin_curve, &other).is_ok());
        assert_eq!(
            union(&with(&thin_curve, &far_triangle(far)), &other).unwrap_err(),
            Error::FeatureBelowTolerance { eps: auto_eps(far) }
        );

        // One box per subpath, with the closing segment.
        let mut open_close = BezPath::new();
        open_close.move_to((0.0, 0.0));
        open_close.line_to((5.0, 1.0));
        open_close.close_path();
        open_close.line_to((0.0, 3.0));
        open_close.close_path();
        open_close.move_to((10.0, 10.0));
        open_close.line_to((12.0, 11.0));
        open_close.close_path();
        assert_eq!(
            subpath_bounding_boxes(&open_close),
            vec![
                kurbo::Rect::new(0.0, 0.0, 5.0, 3.0),
                kurbo::Rect::new(10.0, 10.0, 12.0, 11.0)
            ]
        );
    }
}
